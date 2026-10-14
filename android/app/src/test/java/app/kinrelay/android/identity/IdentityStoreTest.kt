package app.kinrelay.android.identity

import java.io.File
import java.nio.file.Files
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class IdentityStoreTest {
    /** A software AES-GCM stand-in for the Keystore key, with the same file format. */
    private class SoftwareWrapper(private val key: ByteArray = ByteArray(32).also(SecureRandom()::nextBytes)) : KeyWrapper {
        override fun encrypt(plain: ByteArray): ByteArray {
            val iv = ByteArray(12).also(SecureRandom()::nextBytes)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, iv))
            return byteArrayOf(1) + iv + cipher.doFinal(plain)
        }

        override fun decrypt(wrapped: ByteArray): ByteArray {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, wrapped, 1, 12))
            return cipher.doFinal(wrapped, 13, wrapped.size - 13)
        }
    }

    private fun tempDir(): File = Files.createTempDirectory("kin-identity").toFile().also { it.deleteOnExit() }

    @Test
    fun firstRunCreatesAndLaterRunsReuseTheSameIdentity() {
        val dir = tempDir()
        val wrapper = SoftwareWrapper()
        val first = IdentityStore(dir, wrapper).loadOrCreate()
        val second = IdentityStore(dir, wrapper).loadOrCreate()
        assertEquals(first.deviceId, second.deviceId)
        assertArrayEquals(first.certificateDer, second.certificateDer)
        assertArrayEquals(first.privateKey.encoded, second.privateKey.encoded)
    }

    @Test
    fun thePrivateKeyIsNotOnDiskInTheClear() {
        val dir = tempDir()
        val identity = IdentityStore(dir, SoftwareWrapper()).loadOrCreate()
        val stored = File(dir, "identity.key").readBytes()
        val pkcs8 = identity.privateKey.encoded
        assertFalse(stored.toList().windowed(16).any { it == pkcs8.toList().takeLast(16) })
        assertTrue(stored.size > pkcs8.size)
    }

    @Test
    fun anUnreadableKeyFileCreatesANewIdentity() {
        val dir = tempDir()
        val first = IdentityStore(dir, SoftwareWrapper()).loadOrCreate()
        val messages = mutableListOf<String>()
        val second = IdentityStore(dir, SoftwareWrapper()) { messages += it }.loadOrCreate() // another wrapping key
        assertNotEquals(first.deviceId, second.deviceId)
        assertEquals(1, messages.size)
    }
}
