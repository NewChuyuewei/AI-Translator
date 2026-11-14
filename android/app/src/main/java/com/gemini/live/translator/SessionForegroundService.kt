package com.gemini.live.translator

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import com.gemini.live.translator.data.AudioSourceMode
import com.gemini.live.translator.data.SessionState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

class SessionForegroundService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private lateinit var overlay: OverlayCaptionController
    private val runtime: SessionRuntime get() = (application as TranslatorApplication).sessionRuntime

    override fun onCreate() {
        super.onCreate()
        createChannel()
        overlay = OverlayCaptionController(this) { runtime.stop() }
        scope.launch {
            runtime.session.collect { state ->
                val text = when (state) {
                    SessionState.Idle -> "会话已结束"
                    SessionState.Connecting -> "正在安全连接 Gemini…"
                    SessionState.Running -> "正在采集和处理语音"
                    SessionState.Stopping -> "正在停止…"
                    is SessionState.Failed -> state.safeMessage
                }
                getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification(text))
                syncOverlay()
                if (state is SessionState.Idle || state is SessionState.Failed) stopSelf()
            }
        }
        scope.launch {
            runtime.caption.collect { caption -> overlay.update(caption) }
        }
        scope.launch {
            runtime.overlayEnabled.collect { syncOverlay() }
        }
    }

    private fun syncOverlay() {
        val config = runtime.activeSessionConfig()
        if (runtime.session.value is SessionState.Running && runtime.overlayEnabled.value && config != null) {
            overlay.show(config.targetLanguage, config.audioSource, config.mode.transcriptionOnly)
            overlay.update(runtime.caption.value)
        } else {
            overlay.hide()
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            runtime.stop()
            stopSelf()
            return START_NOT_STICKY
        }
        val mode = intent?.getStringExtra(EXTRA_AUDIO_SOURCE)
            ?.let { runCatching { AudioSourceMode.valueOf(it) }.getOrNull() }
            ?: AudioSourceMode.MICROPHONE
        showForeground("正在准备实时翻译…", mode)
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        if (::overlay.isInitialized) overlay.hide()
        scope.cancel()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun showForeground(text: String, mode: AudioSourceMode) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            var types = 0
            if (mode.usesMicrophone) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            if (mode.usesDeviceAudio) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
            startForeground(NOTIFICATION_ID, notification(text), types)
        } else {
            startForeground(NOTIFICATION_ID, notification(text))
        }
    }

    private fun notification(text: String): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val stop = PendingIntent.getService(
            this,
            1,
            Intent(this, SessionForegroundService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, "停止", stop)
            .build()
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(CHANNEL_ID, "实时翻译会话", NotificationManager.IMPORTANCE_LOW).apply {
                description = "麦克风采集与实时翻译状态"
                setShowBadge(false)
            }
            getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        }
    }

    companion object {
        private const val CHANNEL_ID = "live_session"
        private const val NOTIFICATION_ID = 1001
        private const val ACTION_STOP = "com.gemini.live.translator.STOP"
        private const val EXTRA_AUDIO_SOURCE = "audio_source"

        fun start(context: Context, mode: AudioSourceMode) {
            val intent = Intent(context, SessionForegroundService::class.java)
                .putExtra(EXTRA_AUDIO_SOURCE, mode.name)
            ContextCompat.startForegroundService(context, intent)
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, SessionForegroundService::class.java))
        }
    }
}
