package com.gemini.livetranslator.audio

import android.app.*
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.media.*
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.*
import android.provider.Settings
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.max

/** User-initiated, non-sticky foreground service. Android may still stop it. */
class AudioCaptureService : Service() {
    companion object {
        @Volatile var current: AudioCaptureService? = null
        @Volatile private var stoppedThrough = 0L
        private data class Pending(val id: Long, val callback: (String?) -> Unit)
        private var pending: Pending? = null
        @Synchronized fun cancelThrough(id: Long) { stoppedThrough = max(stoppedThrough, id) }
        fun isCancelled(id: Long): Boolean = id <= stoppedThrough
        @Synchronized fun setPending(id: Long, callback: (String?) -> Unit): Boolean {
            if (pending != null) return false
            pending = Pending(id, callback); return true
        }
        fun finishPending(id: Long, error: String?) {
            val callback = synchronized(this) {
                if (pending?.id == id) pending.also { pending = null }?.callback else null
            }
            callback?.invoke(error)
        }
        private const val RATE = 16_000
        private const val NOTIFICATION_ID = 1101
        private const val CHANNEL_ID = "live_translation"
    }
    @Volatile var sessionId: Long = 0
        private set
    private val running = AtomicBoolean(false)
    private val main = Handler(Looper.getMainLooper())
    private val records = mutableListOf<AudioRecord>()
    private var projection: MediaProjection? = null
    private var wakeLock: PowerManager.WakeLock? = null
    private var overlay: LinearLayout? = null
    private var inputText: TextView? = null
    private var outputText: TextView? = null
    private var overlayParams: WindowManager.LayoutParams? = null
    private var transcriptionOnly = false
    private var editable = false
    private lateinit var windowManager: WindowManager

    override fun onBind(intent: Intent?): IBinder? = null
    override fun onCreate() { super.onCreate(); current = this; windowManager = getSystemService(WINDOW_SERVICE) as WindowManager }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent == null) { stopSelf(); return START_NOT_STICKY }
        val requestedId = intent.getLongExtra("sessionId", 0)
        if (requestedId <= 0 || isCancelled(requestedId)) {
            finishPending(requestedId, "启动已取消")
            if (!running.get()) stopSelf(startId)
            return START_NOT_STICKY
        }
        if (running.get()) { finishPending(requestedId, "采集已在运行"); return START_NOT_STICKY }
        sessionId = requestedId
        transcriptionOnly = intent.getBooleanExtra("transcriptionOnly", false)
        val mode = intent.getStringExtra("mode") ?: "microphone_only"
        val wantsPlayback = mode != "microphone_only"
        val wantsMic = mode != "system_audio_only"
        try {
            check(mode in listOf("microphone_only", "system_audio_only", "mixed")) { "无效音源模式" }
            // Android 14+: consent is obtained in Activity, then the FGS is
            // promoted, and only then is getMediaProjection called.
            val notification = buildNotification()
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                var types = 0
                if (wantsMic) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
                if (wantsPlayback) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
                startForeground(NOTIFICATION_ID, notification, types)
            } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && wantsPlayback) {
                startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
            } else startForeground(NOTIFICATION_ID, notification)

            if (wantsPlayback) {
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) throw IllegalStateException("系统内录需要 Android 10+")
                @Suppress("DEPRECATION")
                val data = intent.getParcelableExtra<Intent>("projectionData") ?: throw IllegalStateException("缺少系统录屏授权")
                val manager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
                projection = manager.getMediaProjection(intent.getIntExtra("resultCode", Activity.RESULT_CANCELED), data)
                    ?: throw IllegalStateException("无法获得 MediaProjection")
                projection!!.registerCallback(object : MediaProjection.Callback() {
                    override fun onStop() {
                        if (running.get()) {
                            NativeAudioBridge.reportError("系统音频捕获授权被撤销")
                            stopCapture(); stopSelf()
                        }
                    }
                }, main)
            }
            // Register each recorder immediately so a failure opening the next
            // source still releases the first one in the catch block.
            val mic = if (wantsMic) makeMicrophone().also { records.add(it) } else null
            val playback = if (wantsPlayback) makePlayback(projection!!).also { records.add(it) } else null
            running.set(true)
            for (record in records) {
                record.startRecording()
                check(record.recordingState == AudioRecord.RECORDSTATE_RECORDING) { "无法启动 AudioRecord" }
            }
            // Bounded wake lock; notification remains visible until the user stops.
            val pm = getSystemService(POWER_SERVICE) as PowerManager
            wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "GeminiTranslator:LiveAudio").apply { acquire(30 * 60 * 1000L) }
            if (Settings.canDrawOverlays(this)) {
                // Overlay permission can be revoked between the check and addView;
                // failure of the optional overlay must not break audio capture.
                try { createOverlay() } catch (e: Exception) { android.util.Log.w("LiveTranslator", "无法显示字幕悬浮窗", e) }
            }
            if (mic != null) startReader(mic, false)
            if (playback != null) startReader(playback, true)
            finishPending(sessionId, null)
        } catch (e: Exception) {
            finishPending(sessionId, "启动 Android 音频采集失败: ${e.message}")
            stopCapture(); stopSelf(startId)
        }
        return START_NOT_STICKY
    }
    private fun format(): AudioFormat = AudioFormat.Builder()
        .setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(RATE)
        .setChannelMask(AudioFormat.CHANNEL_IN_MONO).build()
    private fun bufferBytes(): Int {
        val minimum = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        check(minimum > 0) { "设备不支持 16 kHz 单声道采集" }
        return max(minimum * 2, RATE / 5 * 2)
    }
    private fun checkedRecord(record: AudioRecord, message: String): AudioRecord {
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            record.release(); throw IllegalStateException(message)
        }
        return record
    }
    private fun makeMicrophone(): AudioRecord = checkedRecord(AudioRecord.Builder()
        .setAudioSource(MediaRecorder.AudioSource.MIC).setAudioFormat(format())
        .setBufferSizeInBytes(bufferBytes()).build(), "麦克风初始化失败")
    private fun makePlayback(p: MediaProjection): AudioRecord {
        check(Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q)
        val capture = AudioPlaybackCaptureConfiguration.Builder(p)
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .excludeUid(android.os.Process.myUid()) // don't re-capture our own translated speech
            .build()
        return checkedRecord(AudioRecord.Builder().setAudioFormat(format()).setBufferSizeInBytes(bufferBytes())
            .setAudioPlaybackCaptureConfig(capture).build(), "系统音频初始化失败")
    }
    private fun startReader(record: AudioRecord, system: Boolean) {
        Thread({
            val buffer = ShortArray(1600)
            try {
                while (running.get()) {
                    val n = record.read(buffer, 0, buffer.size, AudioRecord.READ_BLOCKING)
                    if (n > 0) NativeAudioBridge.pushAudio(system, buffer, n, RATE)
                    else if (n < 0 && running.get()) throw IllegalStateException("AudioRecord.read=$n")
                }
            } catch (e: Exception) {
                if (running.get()) {
                    NativeAudioBridge.reportError("Android 音频采集已中断: ${e.message}")
                    main.post { stopCapture(); stopSelf() }
                }
            }
        }, if (system) "system-audio-reader" else "microphone-reader").start()
    }
    private fun buildNotification(): Notification {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(CHANNEL_ID, "实时字幕", NotificationManager.IMPORTANCE_LOW)
            (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).createNotificationChannel(channel)
        }
        val launch = packageManager.getLaunchIntentForPackage(packageName)
        val pending = if (launch != null) PendingIntent.getActivity(this, 0, launch,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE) else null
        return Notification.Builder(this, CHANNEL_ID).setContentTitle(if (transcriptionOnly) "Gemini 实时转写运行中" else "Gemini 实时翻译运行中")
            .setContentText("正在采集音频并发送到 Gemini；打开应用可停止")
            .setSmallIcon(android.R.drawable.ic_btn_speak_now).setOngoing(true).setContentIntent(pending).build()
    }
    private fun text(size: Float, color: Int): TextView = TextView(this).apply {
        textSize = size; setTextColor(color); maxLines = 2; setPadding(14, 3, 14, 3)
    }
    private fun createOverlay() {
        if (overlay != null || !Settings.canDrawOverlays(this)) return
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            background = GradientDrawable().apply { setColor(0xDF151D31.toInt()); cornerRadius = 20f }
        }
        if (transcriptionOnly) {
            root.addView(text(11f, Color.LTGRAY).apply { text = "实时转写 · 原文" })
            inputText = text(19f, Color.WHITE).also { it.text = "等待语音输入…"; it.setTypeface(null, Typeface.BOLD); root.addView(it) }
            outputText = null
        } else {
            inputText = text(14f, Color.LTGRAY).also { it.text = "等待原文…"; root.addView(it) }
            outputText = text(19f, Color.WHITE).also { it.text = "等待翻译…"; it.setTypeface(null, Typeface.BOLD); root.addView(it) }
        }
        val lp = WindowManager.LayoutParams(WindowManager.LayoutParams.WRAP_CONTENT, WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE,
            PixelFormat.TRANSLUCENT).apply { gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL; x = 0; y = 120; alpha = 0.75f }
        var startX = 0; var startY = 0; var touchX = 0f; var touchY = 0f
        root.setOnTouchListener { _: View, event: MotionEvent ->
            if (!editable) false else when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> { startX = lp.x; startY = lp.y; touchX = event.rawX; touchY = event.rawY; true }
                MotionEvent.ACTION_MOVE -> { lp.x = startX + (event.rawX - touchX).toInt(); lp.y = startY + (event.rawY - touchY).toInt(); windowManager.updateViewLayout(root, lp); true }
                else -> true
            }
        }
        windowManager.addView(root, lp); overlay = root; overlayParams = lp
    }
    fun setCaption(input: String, output: String) { main.post {
        inputText?.text = input.takeLast(300).ifBlank { if (transcriptionOnly) "等待语音输入…" else "等待原文…" }
        if (!transcriptionOnly) outputText?.text = output.takeLast(300).ifBlank { "等待翻译…" }
    } }
    fun setOverlayEditable(value: Boolean) { main.post {
        if (overlay == null && Settings.canDrawOverlays(this)) createOverlay()
        editable = value
        val view = overlay; val params = overlayParams
        if (view != null && params != null) {
            params.flags = WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                (if (editable) WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL else WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE)
            try { windowManager.updateViewLayout(view, params) } catch (_: Exception) {}
        }
    } }
    fun stopCapture() {
        running.set(false)
        for (record in records) { try { record.stop() } catch (_: Exception) {}; try { record.release() } catch (_: Exception) {} }
        records.clear()
        try { projection?.stop() } catch (_: Exception) {}; projection = null
        try { if (wakeLock?.isHeld == true) wakeLock?.release() } catch (_: Exception) {}; wakeLock = null
        main.post {
            overlay?.let { try { windowManager.removeView(it) } catch (_: Exception) {} }
            overlay = null; overlayParams = null; inputText = null; outputText = null
        }
    }
    override fun onDestroy() {
        if (running.get() && !isCancelled(sessionId)) NativeAudioBridge.reportError("Android 前台采集服务已停止")
        finishPending(sessionId, "前台服务已停止")
        stopCapture(); if (current === this) current = null
        super.onDestroy()
    }
}
