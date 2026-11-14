package com.gemini.live.translator.security

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

class SecretStore(context: Context) {
    private val preferences = context.getSharedPreferences("secure_session", Context.MODE_PRIVATE)
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    fun saveApiKey(value: String) {
        val plaintext = value.encodeToByteArray()
        try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.ENCRYPT_MODE, key())
            val ciphertext = cipher.doFinal(plaintext)
            preferences.edit()
                .putString("api_key_iv", cipher.iv.toBase64())
                .putString("api_key_data", ciphertext.toBase64())
                .apply()
            ciphertext.fill(0)
        } finally {
            plaintext.fill(0)
        }
    }

    fun loadApiKey(): String? {
        val iv = preferences.getString("api_key_iv", null)?.fromBase64() ?: return null
        val encrypted = preferences.getString("api_key_data", null)?.fromBase64() ?: return null
        return try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, iv))
            val plaintext = cipher.doFinal(encrypted)
            try { plaintext.decodeToString() } finally { plaintext.fill(0) }
        } catch (_: Exception) {
            null
        } finally {
            iv.fill(0)
            encrypted.fill(0)
        }
    }

    fun clear() {
        preferences.edit().clear().apply()
    }

    private fun key(): SecretKey {
        (keyStore.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(
                KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setKeySize(256)
                    .build(),
            )
            generateKey()
        }
    }

    private fun ByteArray.toBase64(): String = android.util.Base64.encodeToString(this, android.util.Base64.NO_WRAP)
    private fun String.fromBase64(): ByteArray = android.util.Base64.decode(this, android.util.Base64.NO_WRAP)

    private companion object {
        const val ALIAS = "gemini-live-api-key-v1"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
    }
}
