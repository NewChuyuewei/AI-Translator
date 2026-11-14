package com.gemini.live.translator.ui

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.gemini.live.translator.data.AudioSourceMode
import com.gemini.live.translator.data.GeminiMode
import com.gemini.live.translator.data.SessionState

@Composable
fun TranslatorScreen(viewModel: MainViewModel) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var authorizing by remember { mutableStateOf(false) }
    val requiredPermissions = remember {
        buildList {
            add(Manifest.permission.RECORD_AUDIO)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) add(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
    val projectionRequest = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        authorizing = false
        if (result.resultCode == Activity.RESULT_OK && result.data != null) {
            viewModel.start(result.data)
        } else {
            viewModel.projectionPermissionDenied()
        }
    }
    fun continueAfterPermissions() {
        if (state.audioSource.usesDeviceAudio) {
            val manager = context.getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
            projectionRequest.launch(manager.createScreenCaptureIntent())
        } else {
            authorizing = false
            viewModel.start()
        }
    }
    val permissionRequest = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
        if (requiredPermissions.all { grants[it] == true }) {
            continueAfterPermissions()
        } else {
            authorizing = false
            viewModel.permissionDenied()
        }
    }
    val active = state.session is SessionState.Connecting ||
        state.session is SessionState.Running ||
        state.session is SessionState.Stopping
    val controlsLocked = active || authorizing

    Scaffold { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState()),
        ) {
            Header(state.session)
            Column(
                modifier = Modifier.padding(horizontal = 18.dp, vertical = 20.dp),
                verticalArrangement = Arrangement.spacedBy(18.dp),
            ) {
                SectionTitle("连接", "API Key 仅通过加密连接发送")
                OutlinedTextField(
                    value = state.apiKey,
                    onValueChange = viewModel::setApiKey,
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text("Gemini API Key") },
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                    singleLine = true,
                    enabled = !controlsLocked,
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(
                        checked = state.rememberKey,
                        onCheckedChange = viewModel::setRememberKey,
                        enabled = !controlsLocked,
                    )
                    Column {
                        Text("加密保存到此设备", style = MaterialTheme.typography.bodyMedium)
                        Text("由 Android Keystore 保护", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }

                HorizontalDivider()
                SectionTitle("模型", "选择翻译或纯语音转写")
                GeminiMode.entries.forEach { mode ->
                    SelectionCard(
                        title = mode.displayName,
                        description = when (mode) {
                            GeminiMode.GENERAL_LIVE -> "通用低延迟译员模式，按指令翻译到目标语言"
                            GeminiMode.EXTENDED_THINKING -> "增强推理译员模式，质量更高但可能增加延迟"
                            GeminiMode.LIVE_TRANSLATE -> "专用单目标语言翻译，显示原文与译文"
                            GeminiMode.LIVE_TRANSCRIBE -> "只显示识别原文，不翻译、不播放译音"
                        },
                        selected = state.mode == mode,
                        enabled = !controlsLocked,
                        badge = if (mode.transcriptionOnly) "转写" else "翻译",
                        onClick = { viewModel.setMode(mode) },
                    )
                }
                if (!state.mode.transcriptionOnly) {
                    OutlinedTextField(
                        value = state.targetLanguage,
                        onValueChange = viewModel::setTargetLanguage,
                        modifier = Modifier.fillMaxWidth(),
                        label = { Text("目标语言代码") },
                        supportingText = { Text("BCP-47，例如 zh-Hans、en、ja") },
                        singleLine = true,
                        enabled = !controlsLocked,
                    )
                }

                HorizontalDivider()
                SectionTitle("收听来源", "手机内声音需要 Android 10+ 和每次会话授权")
                AudioSourceMode.entries.forEach { source ->
                    SelectionCard(
                        title = source.displayName,
                        description = when (source) {
                            AudioSourceMode.MICROPHONE -> "收听你和设备附近的声音"
                            AudioSourceMode.DEVICE_AUDIO -> "捕获允许录制的媒体与游戏声音"
                            AudioSourceMode.MIXED -> "将麦克风与手机内声音实时混合"
                        },
                        selected = state.audioSource == source,
                        enabled = !controlsLocked && (!source.usesDeviceAudio || Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q),
                        badge = when (source) {
                            AudioSourceMode.MICROPHONE -> "MIC"
                            AudioSourceMode.DEVICE_AUDIO -> "DEVICE"
                            AudioSourceMode.MIXED -> "MIX"
                        },
                        onClick = { viewModel.setAudioSource(source) },
                    )
                }
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
                    Text(
                        "当前 Android 版本不支持手机内声音捕获。",
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodySmall,
                    )
                }

                Button(
                    onClick = {
                        if (active) {
                            viewModel.stop()
                        } else {
                            authorizing = true
                            if (requiredPermissions.all { permission ->
                                    ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
                                }) {
                                continueAfterPermissions()
                            } else {
                                permissionRequest.launch(requiredPermissions.toTypedArray())
                            }
                        }
                    },
                    modifier = Modifier.fillMaxWidth().height(54.dp),
                    enabled = !authorizing && state.session !is SessionState.Stopping,
                    shape = RoundedCornerShape(16.dp),
                ) {
                    Text(
                        when {
                            authorizing -> "等待系统授权…"
                            active -> "停止会话"
                            state.mode.transcriptionOnly -> "开始转写"
                            else -> "开始翻译"
                        },
                        fontWeight = FontWeight.SemiBold,
                    )
                }

                SessionStatus(state.session, state.audioSource)
                SectionTitle("实时字幕", if (active) "正在显示最新识别结果" else "开始会话后在此显示")
                CaptionCard("原文", state.caption.original.ifEmpty { "等待语音…" })
                if (!state.mode.transcriptionOnly) {
                    CaptionCard("译文", state.caption.translated.ifEmpty { "等待翻译…" }, emphasized = true)
                }
                Spacer(Modifier.height(24.dp))
            }
        }
    }
}

@Composable
private fun Header(session: SessionState) {
    val colors = listOf(MaterialTheme.colorScheme.primary, MaterialTheme.colorScheme.tertiary)
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .background(Brush.linearGradient(colors))
            .padding(horizontal = 22.dp, vertical = 28.dp),
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(
                "Gemini Live Translator",
                style = MaterialTheme.typography.headlineSmall,
                color = MaterialTheme.colorScheme.onPrimary,
                fontWeight = FontWeight.Bold,
            )
            Text(
                if (session is SessionState.Running) "实时会话运行中" else "原生 Kotlin · 安全低延迟音频",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.86f),
            )
        }
    }
}

@Composable
private fun SectionTitle(title: String, subtitle: String) {
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
        Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun SelectionCard(
    title: String,
    description: String,
    selected: Boolean,
    enabled: Boolean,
    badge: String,
    onClick: () -> Unit,
) {
    ElevatedCard(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, onClick = onClick),
        colors = CardDefaults.elevatedCardColors(
            containerColor = if (selected) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainer,
            disabledContainerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.45f),
        ),
        border = if (selected) BorderStroke(1.5.dp, MaterialTheme.colorScheme.primary) else null,
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(
                color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.secondaryContainer,
                shape = RoundedCornerShape(10.dp),
            ) {
                Text(
                    badge,
                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 7.dp),
                    style = MaterialTheme.typography.labelSmall,
                    color = if (selected) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSecondaryContainer,
                    fontWeight = FontWeight.Bold,
                )
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(title, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
                Text(description, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Text(if (selected) "●" else "○", color = MaterialTheme.colorScheme.primary)
        }
    }
}

@Composable
private fun SessionStatus(state: SessionState, source: AudioSourceMode) {
    val text = when (state) {
        SessionState.Idle -> "准备就绪 · ${source.displayName}"
        SessionState.Connecting -> "正在安全连接 Gemini…"
        SessionState.Running -> "会话已连接 · ${source.displayName}"
        SessionState.Stopping -> "正在停止并清理音频…"
        is SessionState.Failed -> state.safeMessage
    }
    Surface(
        color = if (state is SessionState.Failed) MaterialTheme.colorScheme.errorContainer else MaterialTheme.colorScheme.secondaryContainer,
        shape = RoundedCornerShape(12.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(
            text,
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 12.dp),
            color = if (state is SessionState.Failed) MaterialTheme.colorScheme.onErrorContainer else MaterialTheme.colorScheme.onSecondaryContainer,
            style = MaterialTheme.typography.bodyMedium,
        )
    }
}

@Composable
private fun CaptionCard(title: String, value: String, emphasized: Boolean = false) {
    ElevatedCard(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.elevatedCardColors(
            containerColor = if (emphasized) MaterialTheme.colorScheme.tertiaryContainer else MaterialTheme.colorScheme.surfaceContainer,
        ),
    ) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(9.dp)) {
            Text(title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
            Text(value, style = MaterialTheme.typography.bodyLarge)
        }
    }
}
