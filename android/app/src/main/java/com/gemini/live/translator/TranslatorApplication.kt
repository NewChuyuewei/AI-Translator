package com.gemini.live.translator

import android.app.Application

class TranslatorApplication : Application() {
    val sessionRuntime: SessionRuntime by lazy(LazyThreadSafetyMode.SYNCHRONIZED) { SessionRuntime(this) }
}
