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

// AGP still exposes an older JvmTarget enum to build scripts. Pass the target
// directly to the aligned Kotlin 2.4 compiler without linking that stale enum.
tasks.withType<KotlinCompile>().configureEach {
    compilerOptions {
        freeCompilerArgs.addAll("-jvm-target", "25")
    }
}

dependencies {
    implementation(project(":tauri-android"))
    implementation("androidx.core:core-ktx:1.18.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
}
