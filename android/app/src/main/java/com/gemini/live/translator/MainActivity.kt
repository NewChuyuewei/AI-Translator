package com.gemini.live.translator

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import com.gemini.live.translator.security.SecretStore
import com.gemini.live.translator.ui.MainViewModel
import com.gemini.live.translator.ui.TranslatorScreen

class MainActivity : ComponentActivity() {
    private val viewModel: MainViewModel by viewModels {
        object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T {
                require(modelClass == MainViewModel::class.java)
                val runtime = (application as TranslatorApplication).sessionRuntime
                return MainViewModel(runtime, SecretStore(applicationContext)) as T
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        setContent {
            androidx.compose.material3.MaterialTheme {
                TranslatorScreen(viewModel)
            }
        }
    }
}
