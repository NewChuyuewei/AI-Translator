package com.gemini.livetranslator.audio

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.result.ActivityResult
import app.tauri.PermissionState
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.net.InetSocketAddress
import java.net.Proxy
import java.net.ProxySelector
import java.net.URI

@InvokeArg class StartArgs { var mode: String = "microphone_only"; var sessionId: Long = 0; var transcriptionOnly: Boolean = false }
@InvokeArg class StopArgs { var sessionId: Long = 0 }
@InvokeArg class EditArgs { var editable: Boolean = false }
@InvokeArg class CaptionArgs { var input: String = ""; var output: String = "" }
@InvokeArg class ProxyArgs { var url: String = "" }
@InvokeArg class ClipboardArgs { var text: String = "" }

@TauriPlugin(permissions = [Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = "microphone")])
class AudioPlugin(private val activity: Activity) : Plugin(activity) {
    private var startMode = "microphone_only"
    private var startTranscriptionOnly = false
    private var startId = 0L
    private var activeServiceId = 0L
    private var startPending = false
    private var overlayPending = false

    // Use the platform clipboard rather than relying on navigator.clipboard,
    // which may be unavailable to Android WebViews on non-secure origins.
    @Command fun copyDiagnosticText(invoke: Invoke) {
        val text = invoke.parseArgs(ClipboardArgs::class.java).text
        if (text.length > 512 * 1024) { invoke.reject("日志过长，请先清空后重试"); return }
        activity.runOnUiThread {
            try {
                val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                clipboard.setPrimaryClip(ClipData.newPlainText("Gemini 连接诊断日志", text))
                invoke.resolve(JSObject())
            } catch (_: Exception) { invoke.reject("剪贴板不可用，请手动选择日志复制") }
        }
    }

    // ProxySelector applies the Android network's HTTP/PAC and bypass settings.
    // A VPN that routes the app instead needs no explicit proxy here. Run the
    // potentially blocking PAC selection away from the Activity/UI thread.
    @Command fun systemProxy(invoke: Invoke) {
        val rawUrl = invoke.parseArgs(ProxyArgs::class.java).url
        Thread({
            try {
                val target = URI(rawUrl)
                if (target.scheme != "https" || target.host != "generativelanguage.googleapis.com" || target.rawQuery != null) {
                    invoke.reject("无效的代理检测目标"); return@Thread
                }
                val selected = ProxySelector.getDefault()?.select(target)?.firstOrNull()
                if (selected == null) { invoke.reject("系统代理选择器未返回路径"); return@Thread }
                val result = JSObject()
                when (selected.type()) {
                    Proxy.Type.DIRECT -> result.put("kind", "direct")
                    Proxy.Type.HTTP, Proxy.Type.SOCKS -> {
                        val address = selected.address() as? InetSocketAddress
                        if (address == null || address.port !in 1..65535 || address.hostString.isNullOrBlank()) {
                            invoke.reject("系统代理地址无效"); return@Thread
                        }
                        result.put("kind", if (selected.type() == Proxy.Type.HTTP) "http" else "socks5")
                        result.put("host", address.hostString)
                        result.put("port", address.port)
                    }
                    else -> { invoke.reject("不支持的系统代理类型"); return@Thread }
                }
                invoke.resolve(result)
            } catch (_: Exception) { invoke.reject("无法读取 Android 系统代理") }
        }, "system-proxy-lookup").start()
    }

    private fun cancelled(): Boolean = AudioCaptureService.isCancelled(startId)
    private fun rejectStart(invoke: Invoke, message: String) {
        startPending = false
        invoke.reject(message)
    }

    @Command fun startCapture(invoke: Invoke) {
        val args = invoke.parseArgs(StartArgs::class.java)
        if (args.mode !in listOf("microphone_only", "system_audio_only", "mixed") || args.sessionId <= 0) {
            invoke.reject("无效的音源模式或会话编号"); return
        }
        // A stop may arrive before startCapture is dispatched to Android. Never
        // allow a cancelled session's late start to record in the background.
        if (AudioCaptureService.isCancelled(args.sessionId)) { invoke.reject("启动已取消"); return }
        // Tauri's activity-result launcher is shared: do not overlap projection
        // and overlay permission requests or overwrite a pending callback.
        if (startPending || overlayPending || AudioCaptureService.current != null) {
            invoke.reject("采集或授权窗口正在处理，请先完成或关闭授权窗口"); return
        }
        if (args.mode != "microphone_only" && Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            invoke.reject("系统内录需要 Android 10 (API 29) 或更高版本"); return
        }
        startPending = true; startId = args.sessionId; startMode = args.mode; startTranscriptionOnly = args.transcriptionOnly
        try {
            // PlaybackCapture also requires RECORD_AUDIO, even without a mic stream.
            if (getPermissionState("microphone") != PermissionState.GRANTED) {
                requestPermissionForAlias("microphone", invoke, "onMicPermission")
            } else requestProjectionOrStart(invoke)
        } catch (e: Exception) { rejectStart(invoke, "请求 Android 权限失败: ${e.message}") }
    }

    @PermissionCallback fun onMicPermission(invoke: Invoke) {
        if (cancelled()) { rejectStart(invoke, "启动已取消"); return }
        if (getPermissionState("microphone") != PermissionState.GRANTED) {
            rejectStart(invoke, "用户未授权录音"); return
        }
        try { requestProjectionOrStart(invoke) }
        catch (e: Exception) { rejectStart(invoke, "请求系统音频授权失败: ${e.message}") }
    }

    private fun requestProjectionOrStart(invoke: Invoke) {
        if (cancelled()) { rejectStart(invoke, "启动已取消"); return }
        if (startMode == "microphone_only") beginService(invoke, Activity.RESULT_CANCELED, null)
        else {
            val manager = activity.getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
            startActivityForResult(invoke, manager.createScreenCaptureIntent(), "onProjectionResult")
        }
    }

    @ActivityCallback fun onProjectionResult(invoke: Invoke, result: ActivityResult) {
        if (cancelled()) { rejectStart(invoke, "启动已取消"); return }
        if (result.resultCode != Activity.RESULT_OK || result.data == null) {
            rejectStart(invoke, "用户未授权系统音频捕获"); return
        }
        beginService(invoke, result.resultCode, result.data)
    }

    private fun beginService(invoke: Invoke, resultCode: Int, data: Intent?) {
        if (cancelled()) { rejectStart(invoke, "启动已取消"); return }
        val id = startId
        if (!AudioCaptureService.setPending(id) { error ->
            if (startId == id) startPending = false
            if (error == null && !AudioCaptureService.isCancelled(id)) invoke.resolve(JSObject())
            else invoke.reject(error ?: "启动已取消")
        }) { rejectStart(invoke, "另一个采集服务正在启动"); return }
        activeServiceId = id
        try {
            val intent = Intent(activity, AudioCaptureService::class.java)
                .putExtra("mode", startMode).putExtra("sessionId", id).putExtra("resultCode", resultCode)
                .putExtra("transcriptionOnly", startTranscriptionOnly)
            if (data != null) intent.putExtra("projectionData", data)
            activity.startForegroundService(intent)
        } catch (e: Exception) {
            activeServiceId = 0
            AudioCaptureService.finishPending(id, "启动前台服务失败: ${e.message}")
        }
    }

    @Command fun stopCapture(invoke: Invoke) {
        val id = invoke.parseArgs(StopArgs::class.java).sessionId
        if (id <= 0) { invoke.reject("无效的会话编号"); return }
        AudioCaptureService.cancelThrough(id)
        if (activeServiceId == id) {
            AudioCaptureService.current?.let { if (it.sessionId == id) it.stopCapture() }
            activity.stopService(Intent(activity, AudioCaptureService::class.java))
            // Covers cancellation between startForegroundService and onCreate.
            AudioCaptureService.finishPending(id, "启动已取消")
            activeServiceId = 0
        }
        // If a system dialog is still visible, startPending intentionally stays
        // true until its callback arrives, so no later request replaces it.
        invoke.resolve(JSObject())
    }

    @Command fun requestOverlay(invoke: Invoke) {
        if (Settings.canDrawOverlays(activity)) { invoke.resolve(JSObject()); return }
        if (startPending || overlayPending) { invoke.reject("请先完成或关闭当前授权窗口"); return }
        overlayPending = true
        try {
            val intent = Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:${activity.packageName}"))
            startActivityForResult(invoke, intent, "onOverlayResult")
        } catch (e: Exception) { overlayPending = false; invoke.reject("打开悬浮窗设置失败: ${e.message}") }
    }
    @ActivityCallback fun onOverlayResult(invoke: Invoke, result: ActivityResult) {
        overlayPending = false
        if (Settings.canDrawOverlays(activity)) invoke.resolve(JSObject()) else invoke.reject("未授权显示在其他应用上层")
    }
    @Command fun setOverlayEditable(invoke: Invoke) {
        val editable = invoke.parseArgs(EditArgs::class.java).editable
        AudioCaptureService.current?.setOverlayEditable(editable)
        invoke.resolve(JSObject())
    }
    @Command fun caption(invoke: Invoke) {
        val args = invoke.parseArgs(CaptionArgs::class.java)
        AudioCaptureService.current?.setCaption(args.input, args.output)
        invoke.resolve(JSObject())
    }
}
