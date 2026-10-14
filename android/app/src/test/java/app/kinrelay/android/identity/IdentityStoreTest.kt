package app.kinrelay.android.identity

import android.security.keystore.KeyPermanentlyInvalidatedException
import java.io.File
import java.io.IOException
import java.nio.file.Files
import java.security.KeyStoreException
import java.security.ProviderException
import java.security.SecureRandom
import java.security.UnrecoverableKeyException
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class IdentityStoreTest {
    /** A software AES-GCM stand-in for the Keystore key, with the same file format. [failure] makes decrypt fail on demand. */
    private class SoftwareWrapper(
        private val key: ByteArray = ByteArray(32).also(SecureRandom()::nextBytes),
        var failure: (() -> Throwable)? = null,
    ) : KeyWrapper {
        override fun encrypt(plain: ByteArray): ByteArray {
            val iv = ByteArray(12).also(SecureRandom()::nextBytes)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, iv))
            return byteArrayOf(1) + iv + cipher.doFinal(plain)
        }

        override fun decrypt(wrapped: ByteArray): ByteArray {
            failure?.let { throw it() }
            if (wrapped.size <= 13 || wrapped[0] != 1.toByte()) throw CorruptKeyFileException("unknown key file format")
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, wrapped, 1, 12))
            return cipher.doFinal(wrapped, 13, wrapped.size - 13) // a wrong key throws AEADBadTagException
        }
    }

    private fun tempDir(): File = Files.createTempDirectory("kin-identity").toFile().also { it.deleteOnExit() }

    private fun keyFile(dir: File) = File(dir, "identity.key")

    private fun certFile(dir: File) = File(dir, "identity.crt")

    private fun snapshot(dir: File): Map<String, List<Byte>> =
        dir.listFiles().orEmpty().associate { it.name to it.readBytes().toList() }

    // --- the normal paths ---

    @Test
    fun firstRunCreatesAndLaterRunsReuseTheSameIdentity() {
        val dir = tempDir()
        val wrapper = SoftwareWrapper()
        val first = IdentityStore(dir, wrapper).loadOrCreate()
        val second = IdentityStore(dir, wrapper).loadOrCreate()
        assertNull("a first run replaces nothing", first.replaced)
        assertNull(second.replaced)
        assertEquals(first.identity.deviceId, second.identity.deviceId)
        assertArrayEquals(first.identity.certificateDer, second.identity.certificateDer)
        assertArrayEquals(first.identity.privateKey.encoded, second.identity.privateKey.encoded)
    }

    @Test
    fun thePrivateKeyIsNotOnDiskInTheClear() {
        val dir = tempDir()
        val identity = IdentityStore(dir, SoftwareWrapper()).loadOrCreate().identity
        val stored = keyFile(dir).readBytes()
        val pkcs8 = identity.privateKey.encoded
        assertFalse(stored.toList().windowed(16).any { it == pkcs8.toList().takeLast(16) })
        assertTrue(stored.size > pkcs8.size)
    }

    @Test
    fun noTemporaryFileIsLeftBehind() {
        val dir = tempDir()
        IdentityStore(dir, SoftwareWrapper()).loadOrCreate()
        assertEquals(setOf("identity.key", "identity.crt"), dir.list().orEmpty().toSet())
    }

    // --- the identity is replaced only when it is clearly gone or unusable ---

    @Test
    fun aKeyThatCanNoLongerDecryptTheFileCreatesANewIdentity() {
        val dir = tempDir()
        val first = IdentityStore(dir, SoftwareWrapper()).loadOrCreate().identity
        val second = IdentityStore(dir, SoftwareWrapper()).loadOrCreate() // another wrapping key: AEADBadTagException
        assertNotEquals(first.deviceId, second.identity.deviceId)
        assertNotNull(second.replaced)
    }

    @Test
    fun everyKindOfLostKeyReplacesTheIdentity() {
        val lost = listOf<() -> Throwable>(
            { KeyMissingException("alias is gone") },
            { AEADBadTagException("tag mismatch") },
            { UnrecoverableKeyException("key can't be recovered") },
            { KeyPermanentlyInvalidatedException("key was invalidated") },
            { CorruptKeyFileException("unknown format") },
            // wrapped by the provider, as it does:
            { IllegalStateException("operation failed", AEADBadTagException("tag mismatch")) },
        )
        for (failure in lost) {
            val dir = tempDir()
            val wrapper = SoftwareWrapper()
            val first = IdentityStore(dir, wrapper).loadOrCreate().identity
            wrapper.failure = failure
            val second = IdentityStore(dir, wrapper).loadOrCreate()
            assertNotEquals(failure().javaClass.simpleName, first.deviceId, second.identity.deviceId)
            assertNotNull(failure().javaClass.simpleName, second.replaced)
        }
    }

    @Test
    fun aCorruptKeyFileReplacesTheIdentity() {
        val dir = tempDir()
        val wrapper = SoftwareWrapper()
        val first = IdentityStore(dir, wrapper).loadOrCreate().identity
        keyFile(dir).writeBytes(byteArrayOf(9, 9, 9)) // too short, wrong version
        val second = IdentityStore(dir, wrapper).loadOrCreate()
        assertNotEquals(first.deviceId, second.identity.deviceId)
        assertNotNull(second.replaced)
    }

    @Test
    fun aCorruptCertificateReplacesTheIdentity() {
        val dir = tempDir()
        val wrapper = SoftwareWrapper()
        val first = IdentityStore(dir, wrapper).loadOrCreate().identity
        certFile(dir).writeBytes(first.certificateDer.copyOf(40))
        val second = IdentityStore(dir, wrapper).loadOrCreate()
        assertNotEquals(first.deviceId, second.identity.deviceId)
        assertNotNull(second.replaced)
    }

    @Test
    fun aCertificateThatDoesNotBelongToTheKeyIsTreatedAsCorrupt() {
        val dirA = tempDir()
        val dirB = tempDir()
        val wrapper = SoftwareWrapper()
        val a = IdentityStore(dirA, wrapper).loadOrCreate().identity
        val b = IdentityStore(dirB, wrapper).loadOrCreate().identity
        certFile(dirA).writeBytes(b.certificateDer) // A's key with B's certificate: a valid file pair that doesn't match
        val loaded = IdentityStore(dirA, wrapper).loadOrCreate()
        assertNotNull(loaded.replaced)
        assertNotEquals(a.deviceId, loaded.identity.deviceId)
        assertNotEquals(b.deviceId, loaded.identity.deviceId)
    }

    @Test
    fun aKeyWithoutACertificateIsIncomplete() {
        // What a crash between the two writes leaves behind: the key file only.
        val dir = tempDir()
        val wrapper = SoftwareWrapper()
        val first = IdentityStore(dir, wrapper).loadOrCreate().identity
        assertTrue(certFile(dir).delete())
        val second = IdentityStore(dir, wrapper).loadOrCreate()
        assertNotNull(second.replaced)
        assertNotEquals(first.deviceId, second.identity.deviceId)
        assertTrue(certFile(dir).exists() && keyFile(dir).exists())
    }

    // --- a temporary error never touches the files ---

    @Test
    fun aTemporaryKeystoreErrorLeavesTheFilesAndThrows() {
        val temporary = listOf<() -> Throwable>(
            { KeyStoreException("keystore connection error") },
            { ProviderException("Keystore operation failed") },
            { java.security.InvalidKeyException("Keystore not ready") },
            { IOException("EIO") },
            { RuntimeException("something else") },
        )
        for (failure in temporary) {
            val dir = tempDir()
            val wrapper = SoftwareWrapper()
            val first = IdentityStore(dir, wrapper).loadOrCreate().identity
            val before = snapshot(dir)
            wrapper.failure = failure
            val thrown = assertThrows(IdentityUnavailableException::class.java) { IdentityStore(dir, wrapper).loadOrCreate() }
            assertNotNull(thrown.cause)
            assertEquals("files unchanged after ${failure().javaClass.simpleName}", before, snapshot(dir))
            // and when the Keystore is back, the same identity is still there
            wrapper.failure = null
            val again = IdentityStore(dir, wrapper).loadOrCreate()
            assertNull(again.replaced)
            assertEquals(first.deviceId, again.identity.deviceId)
        }
    }

    @Test
    fun aFailedEncryptionOnFirstRunWritesNothing() {
        val dir = tempDir()
        val wrapper = object : KeyWrapper {
            override fun encrypt(plain: ByteArray): ByteArray = throw ProviderException("Keystore busy")
            override fun decrypt(wrapped: ByteArray): ByteArray = error("not used")
        }
        assertThrows(IdentityUnavailableException::class.java) { IdentityStore(dir, wrapper).loadOrCreate() }
        assertEquals(emptyList<String>(), dir.list().orEmpty().toList())
    }

    @Test
    fun anUnreadableFileIsATemporaryErrorNotALostIdentity() {
        val dir = tempDir()
        val wrapper = SoftwareWrapper()
        IdentityStore(dir, wrapper).loadOrCreate()
        val before = snapshot(dir)
        // A directory where the key file should be: reading it fails with an IOException, which may pass.
        assertTrue(keyFile(dir).delete())
        assertTrue(keyFile(dir).mkdir())
        assertThrows(IdentityUnavailableException::class.java) { IdentityStore(dir, wrapper).loadOrCreate() }
        assertEquals(before.getValue("identity.crt"), certFile(dir).readBytes().toList())
    }

    // --- the write order ---

    @Test
    fun theKeyIsWrittenBeforeTheCertificate() {
        val dir = tempDir()
        val order = mutableListOf<String>()
        IdentityStore(dir, SoftwareWrapper()) { file, bytes ->
            order += file.name
            writeAtomically(file, bytes)
        }.loadOrCreate()
        assertEquals(listOf("identity.key", "identity.crt"), order)
    }

    @Test
    fun aCrashBetweenTheTwoWritesIsRecoveredOnTheNextRun() {
        val dir = tempDir()
        val wrapper = SoftwareWrapper()
        val crashing = IdentityStore(dir, wrapper) { file, bytes ->
            if (file.name == "identity.crt") throw IOException("simulated crash")
            writeAtomically(file, bytes)
        }
        assertThrows(IdentityUnavailableException::class.java) { crashing.loadOrCreate() }
        assertTrue(keyFile(dir).exists())
        assertFalse(certFile(dir).exists())
        val recovered = IdentityStore(dir, wrapper).loadOrCreate()
        assertNotNull("incomplete files are replaced", recovered.replaced)
        assertEquals(recovered.identity.deviceId, IdentityStore(dir, wrapper).loadOrCreate().identity.deviceId)
    }

    @Test
    fun theRenameKeepsTheOldFileIntactUntilTheNewOneIsComplete() {
        val dir = tempDir()
        val target = File(dir, "x.bin")
        writeAtomically(target, byteArrayOf(1, 2, 3))
        writeAtomically(target, byteArrayOf(4, 5, 6, 7))
        assertArrayEquals(byteArrayOf(4, 5, 6, 7), target.readBytes())
        assertEquals(listOf("x.bin"), dir.list().orEmpty().toList())
    }

    // --- the classification and the retry steps ---

    @Test
    fun classification() {
        fun kind(e: Throwable) = IdentityFailure.classify(e)
        assertEquals(IdentityFailure.Kind.UNUSABLE, kind(KeyMissingException("x")))
        assertEquals(IdentityFailure.Kind.UNUSABLE, kind(AEADBadTagException()))
        assertEquals(IdentityFailure.Kind.UNUSABLE, kind(UnrecoverableKeyException()))
        assertEquals(IdentityFailure.Kind.UNUSABLE, kind(KeyPermanentlyInvalidatedException("x")))
        assertEquals(IdentityFailure.Kind.UNUSABLE, kind(CorruptKeyFileException("x")))
        assertEquals(IdentityFailure.Kind.UNUSABLE, kind(RuntimeException("outer", AEADBadTagException())))
        assertEquals(IdentityFailure.Kind.TRANSIENT, kind(KeyStoreException("busy")))
        assertEquals(IdentityFailure.Kind.TRANSIENT, kind(ProviderException("busy")))
        assertEquals(IdentityFailure.Kind.TRANSIENT, kind(IOException()))
        assertEquals(IdentityFailure.Kind.TRANSIENT, kind(IllegalArgumentException("not ours to judge")))
    }

    @Test
    fun retryDelaysGrowAndStopGrowing() {
        val steps = (0..8).map(IdentityFailure::retryDelayMillis)
        assertEquals(500L, steps.first())
        assertEquals(60_000L, steps.last())
        assertEquals(steps, steps.sorted())
    }
}
