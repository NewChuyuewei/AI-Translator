package com.gemini.live.translator

import android.content.Context
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.provider.Settings
import android.text.TextUtils
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import com.gemini.live.translator.data.AudioSourceMode
import com.gemini.live.translator.data.CaptionState
import kotlin.math.roundToInt

class OverlayCaptionController(
    private val context: Context,
    private val onStopSession: () -> Unit,
    private val onDismiss: () -> Unit,
) {
    private val windowManager = context.getSystemService(WindowManager::class.java)
    private var root: LinearLayout? = null
    private var original: TextView? = null
    private var translated: TextView? = null
    private var divider: View? = null
    private var collapsed = false
    private var transcriptionOnly = false
    private lateinit var params: WindowManager.LayoutParams

    fun show(targetLanguage: String, source: AudioSourceMode, transcriptionOnly: Boolean) {
        if (root != null || Build.VERSION.SDK_INT < Build.VERSION_CODES.O || !Settings.canDrawOverlays(context)) return
        this.transcriptionOnly = transcriptionOnly
        params = WindowManager.LayoutParams(
            minOf(dp(350), context.resources.displayMetrics.widthPixels - dp(24)),
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL
            y = dp(96)
        }
        val panel = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(14), dp(12), dp(14), dp(14))
            background = roundedBackground(0xe61a1c1e.toInt(), 26f, 0x553f4549)
            elevation = dp(12).toFloat()
        }
        val header = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        header.addView(label("✦", 18f, Color.WHITE).apply {
            gravity = Gravity.CENTER
            background = roundedBackground(0xff2b2e30.toInt(), 50f, 0)
        }, linearParams(46, 46))
        header.addView(label("自动 → ${languageName(targetLanguage)}", 16f, Color.WHITE).apply {
            setPadding(dp(14), 0, dp(8), 0)
            maxLines = 1
            ellipsize = TextUtils.TruncateAt.END
        }, LinearLayout.LayoutParams(0, dp(46), 1f))
        header.addView(actionButton(sourceIcon(source), "当前音源：${source.displayName}") {})
        val collapse = actionButton("↙", "收起或展开悬浮字幕") { toggleCollapsed() }
        header.addView(collapse)
        header.addView(actionButton("■", "停止翻译会话") { onStopSession() })
        header.addView(actionButton("×", "关闭悬浮字幕") {
            onDismiss()
            hide()
        })
        installDrag(header)
        panel.addView(header, LinearLayout.LayoutParams.MATCH_PARENT, dp(48))

        original = captionText("正在聆听${sourceLabel(source)}…", 18f, 0xfff0f1f2.toInt()).also {
            panel.addView(it, LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT)
        }
        divider = View(context).apply {
            setBackgroundColor(0x44ffffff)
            visibility = if (transcriptionOnly) View.GONE else View.VISIBLE
        }.also {
            panel.addView(it, LinearLayout.LayoutParams.MATCH_PARENT, dp(1))
        }
        translated = captionText("等待翻译…", 20f, Color.WHITE).also {
            it.setTypeface(it.typeface, android.graphics.Typeface.BOLD)
            it.visibility = if (transcriptionOnly) View.GONE else View.VISIBLE
            panel.addView(it, LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT)
        }
        root = panel
        runCatching { windowManager.addView(panel, params) }.onFailure { root = null }
    }

    fun update(caption: CaptionState) {
        original?.text = caption.original.ifBlank { "正在聆听…" }
        translated?.text = caption.translated.ifBlank { "等待翻译…" }
    }

    fun hide() {
        val view = root ?: return
        root = null
        original = null
        translated = null
        divider = null
        runCatching { windowManager.removeViewImmediate(view) }
    }

    private fun toggleCollapsed() {
        collapsed = !collapsed
        val visibility = if (collapsed) View.GONE else View.VISIBLE
        original?.visibility = visibility
        translated?.visibility = if (transcriptionOnly) View.GONE else visibility
        divider?.visibility = if (transcriptionOnly) View.GONE else visibility
        root?.let { runCatching { windowManager.updateViewLayout(it, params) } }
    }

    private fun installDrag(handle: View) {
        var initialX = 0
        var initialY = 0
        var downX = 0f
        var downY = 0f
        handle.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    initialX = params.x
                    initialY = params.y
                    downX = event.rawX
                    downY = event.rawY
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    val view = root ?: return@setOnTouchListener false
                    val metrics = context.resources.displayMetrics
                    val panelWidth = view.width.takeIf { it > 0 } ?: params.width
                    val maxX = ((metrics.widthPixels - panelWidth) / 2).coerceAtLeast(0)
                    val maxY = (metrics.heightPixels - view.height - dp(24)).coerceAtLeast(0)
                    params.x = (initialX + (event.rawX - downX).roundToInt()).coerceIn(-maxX, maxX)
                    params.y = (initialY + (event.rawY - downY).roundToInt()).coerceIn(0, maxY)
                    runCatching { windowManager.updateViewLayout(view, params) }
                    true
                }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> true
                else -> false
            }
        }
    }

    private fun captionText(initial: String, size: Float, color: Int) = label(initial, size, color).apply {
        setPadding(dp(4), dp(16), dp(4), dp(16))
        maxLines = 4
        ellipsize = TextUtils.TruncateAt.END
        setLineSpacing(0f, 1.12f)
    }

    private fun actionButton(text: String, description: String, action: () -> Unit): View {
        val container = FrameLayout(context).apply {
            contentDescription = description
            isClickable = true
            isFocusable = true
            background = roundedBackground(0xff2b2e30.toInt(), 50f, 0)
            setOnClickListener { action() }
        }
        container.addView(label(text, 22f, Color.WHITE).apply { gravity = Gravity.CENTER })
        container.layoutParams = linearParams(46, 46).apply { marginStart = dp(6) }
        return container
    }

    private fun label(text: String, size: Float, color: Int) = TextView(context).apply {
        this.text = text
        textSize = size
        setTextColor(color)
        includeFontPadding = false
        gravity = Gravity.CENTER_VERTICAL
    }

    private fun sourceIcon(source: AudioSourceMode): String = when (source) {
        AudioSourceMode.MICROPHONE -> "♩"
        AudioSourceMode.DEVICE_AUDIO -> "⇩"
        AudioSourceMode.MIXED -> "✣"
    }

    private fun sourceLabel(source: AudioSourceMode): String = when (source) {
        AudioSourceMode.MICROPHONE -> "麦克风"
        AudioSourceMode.DEVICE_AUDIO -> "手机内声音"
        AudioSourceMode.MIXED -> "麦克风和手机内声音"
    }

    private fun languageName(code: String): String = when (code.lowercase()) {
        "zh", "zh-hans" -> "中文"
        "zh-hant" -> "繁體中文"
        "en", "en-us", "en-gb" -> "English"
        "ja", "ja-jp" -> "日本語"
        "ko", "ko-kr" -> "한국어"
        "es" -> "Español"
        "fr" -> "Français"
        "de" -> "Deutsch"
        "it" -> "Italiano"
        else -> code
    }

    private fun roundedBackground(color: Int, radiusDp: Float, strokeColor: Int): GradientDrawable =
        GradientDrawable().apply {
            setColor(color)
            cornerRadius = dp(radiusDp.toInt()).toFloat()
            if (strokeColor != 0) setStroke(dp(1), strokeColor)
        }

    private fun linearParams(width: Int, height: Int) =
        LinearLayout.LayoutParams(dp(width), dp(height))

    private fun dp(value: Int): Int = (value * context.resources.displayMetrics.density).roundToInt()
}
