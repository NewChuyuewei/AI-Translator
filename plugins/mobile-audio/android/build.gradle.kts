import com.android.build.api.dsl.LibraryExtension
import org.jetbrains.kotlin.gradle.tasks.KotlinCompile

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

// Configure the public AGP extension directly. Project.android(Action) targets
// the legacy extension and is removed in AGP 10.
extensions.configure<LibraryExtension> {
    namespace = "com.gemini.livetranslator.audio"
    compileSdk = 37
    defaultConfig {
        minSdk = 26
        consumerProguardFiles("consumer-rules.pro")
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_25
        targetCompatibility = JavaVersion.VERSION_25
    }
}

// AGP 9.3.1 exposes its embedded Kotlin 2.2 JvmTarget class to build scripts,
// so linking the Kotlin 2.4-only JVM_25 enum field fails at runtime. The string
// compatibility API still passes target 25 to the Kotlin 2.4 compiler.
tasks.withType<KotlinCompile>().configureEach {
    @Suppress("DEPRECATION", "DEPRECATION_ERROR")
    kotlinOptions.jvmTarget = "25"
}

dependencies {
    implementation(project(":tauri-android"))
    implementation("androidx.core:core-ktx:1.18.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
}
