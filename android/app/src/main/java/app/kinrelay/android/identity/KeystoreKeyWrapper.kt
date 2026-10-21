package app.kinrelay.android.identity

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** AES-256-GCM with a key that never leaves the Android Keystore. Output: version, IV, ciphertext+tag. */
class KeystoreKeyWrapper(private val alias: String = "kin_identity_wrap") : KeyWrapper {
    override fun encrypt(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey(create = true))
        cipher.updateAAD(AAD)
        val iv = cipher.iv
        check(iv.size == IV_BYTES)
        return byteArrayOf(VERSION) + iv + cipher.doFinal(plain)
    }

    override fun decrypt(wrapped: ByteArray): ByteArray {
        if (wrapped.size <= 1 + IV_BYTES || wrapped[0] != VERSION) throw CorruptKeyFileException("unknown key file format")
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, secretKey(create = false), GCMParameterSpec(TAG_BITS, wrapped, 1, IV_BYTES))
        cipher.updateAAD(AAD)
        return cipher.doFinal(wrapped, 1 + IV_BYTES, wrapped.size - 1 - IV_BYTES)
    }

    /** Decrypting must never create the key: a new key could not read the old file and would hide the loss. */
    private fun secretKey(create: Boolean): SecretKey {
        val store = KeyStore.getInstance(PROVIDER).apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        if (!create) throw KeyMissingException("the Keystore key '$alias' does not exist")
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
        generator.init(
            KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return generator.generateKey()
    }

    private companion object {
        const val PROVIDER = "AndroidKeyStore"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val VERSION: Byte = 1
        const val IV_BYTES = 12
        const val TAG_BITS = 128
        val AAD = "kin-identity-v1".toByteArray()
    }
}
