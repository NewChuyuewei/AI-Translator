package com.gemini.live.translator.ui

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.gemini.live.translator.data.GeminiMode
import com.gemini.live.translator.data.SessionState

@Composable
fun TranslatorScreen(viewModel: MainViewModel) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val microphonePermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) viewModel.start() else viewModel.microphonePermissionDenied()
    }
    val active = state.session is SessionState.Connecting || state.session is SessionState.Running || state.session is SessionState.Stopping
    Scaffold { padding ->
        Column(
            modifier = Modifier.fillMaxSize().padding(padding).padding(20.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text("Gemini Live Translator", style = MaterialTheme.typography.headlineMedium)
            Text("原生 Kotlin · JDK 25", style = MaterialTheme.typography.labelLarge)
            OutlinedTextField(
                value = state.apiKey,
                onValueChange = viewModel::setApiKey,
                modifier = Modifier.fillMaxWidth(),
                label = { Text("Gemini API Key") },
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                singleLine = true,
                enabled = !active,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Checkbox(checked = state.rememberKey, onCheckedChange = viewModel::setRememberKey, enabled = !active)
                Text("使用 Android Keystore 加密保存", modifier = Modifier.padding(top = 12.dp))
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(
                    selected = state.mode == GeminiMode.LIVE_TRANSLATE,
                    onClick = { viewModel.setMode(GeminiMode.LIVE_TRANSLATE) },
                    label = { Text("实时翻译") },
                    enabled = !active,
                )
                FilterChip(
                    selected = state.mode == GeminiMode.LIVE_TRANSCRIBE,
                    onClick = { viewModel.setMode(GeminiMode.LIVE_TRANSCRIBE) },
                    label = { Text("仅转写") },
                    enabled = !active,
                )
            }
            OutlinedTextField(
                value = state.targetLanguage,
                onValueChange = viewModel::setTargetLanguage,
                modifier = Modifier.fillMaxWidth(),
                label = { Text("目标语言 BCP-47") },
                singleLine = true,
                enabled = !active && !state.mode.transcriptionOnly,
            )
            Button(
                onClick = {
                    if (active) {
                        viewModel.stop()
                    } else if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                        viewModel.start()
                    } else {
                        microphonePermission.launch(Manifest.permission.RECORD_AUDIO)
                    }
                },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(if (active) "停止" else "开始")
            }
            SessionStatus(state.session)
            CaptionCard("原文", state.caption.original.ifEmpty { "等待语音…" })
            if (!state.mode.transcriptionOnly) CaptionCard("译文", state.caption.translated.ifEmpty { "等待翻译…" })
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun SessionStatus(state: SessionState) {
    val text = when (state) {
        SessionState.Idle -> "准备就绪"
        SessionState.Connecting -> "正在安全连接 Gemini…"
        SessionState.Running -> "会话已连接"
        SessionState.Stopping -> "正在停止…"
        is SessionState.Failed -> state.safeMessage
    }
    Text(text, color = if (state is SessionState.Failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary)
}

@Composable
private fun CaptionCard(title: String, value: String) {
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(title, style = MaterialTheme.typography.labelLarge)
            Text(value, style = MaterialTheme.typography.bodyLarge)
        }
    }
}
