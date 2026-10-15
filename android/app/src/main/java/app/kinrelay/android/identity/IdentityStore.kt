package app.kinrelay.android.identity

import android.os.Build
import android.security.keystore.KeyPermanentlyInvalidatedException
import androidx.annotation.RequiresApi
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.GeneralSecurityException
import java.security.Signature
import java.security.UnrecoverableKeyException
import javax.crypto.AEADBadTagException

/** Encrypts and decrypts the private key bytes. The real one uses the Android Keystore. */
interface KeyWrapper {
    fun encrypt(plain: ByteArray): ByteArray

    /** Must not create a missing wrapping key: a missing key is [KeyMissingException]. */
    fun decrypt(wrapped: ByteArray): ByteArray
}

/** The wrapping key (the Keystore alias) doesn't exist, so the stored private key can never be read again. */
class KeyMissingException(message: String) : GeneralSecurityException(message)

/** The stored key file isn't in the format a [KeyWrapper] wrote. */
class CorruptKeyFileException(message: String) : GeneralSecurityException(message)

/**
 * The identity can't be read or written right now, for a reason that may go away (the Keystore is busy
 * or not ready, an I/O error). Nothing on disk was changed: the caller retries later and tells the user.
 */
class IdentityUnavailableException(message: String, cause: Throwable? = null) : Exception(message, cause)

/**
 * Looks at an exception that sits under an `UnrecoverableKeyException`. `KeyStore.getKey()` wraps the Keystore's own
 * errors in that exception, including temporary ones (the Keystore is busy, a system error), so the type alone doesn't
 * say whether the key is really gone. The real probe reads `android.security.KeyStoreException`; tests pass their own.
 */
fun interface KeystoreCauseProbe {
    /**
     * Null if [error] is not a Keystore error. Otherwise true when the error may pass (or when it can't be told, so the
     * answer is "try again"), false when the Keystore says it will never work.
     */
    fun transientFailure(error: Throwable): Boolean?
}

/** The real probe. `isTransientFailure()` exists from API 33; before that a Keystore error can't be judged and counts as temporary. */
object AndroidKeystoreCauseProbe : KeystoreCauseProbe {
    private const val KEYSTORE_EXCEPTION = "android.security.KeyStoreException"

    override fun transientFailure(error: Throwable): Boolean? {
        // Matched by name: the class is public only from API 33, and an `is` check on a missing class is not safe.
        if (generateSequence<Class<*>>(error.javaClass) { it.superclass }.none { it.name == KEYSTORE_EXCEPTION }) return null
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) isTransient(error) else true
    }

    @RequiresApi(Build.VERSION_CODES.TIRAMISU)
    private fun isTransient(error: Throwable): Boolean = (error as android.security.KeyStoreException).isTransientFailure
}

/** Sorts a failure into "this identity is lost for good" and "try again later". */
object IdentityFailure {
    enum class Kind { UNUSABLE, TRANSIENT }

    /**
     * Only these mean the stored identity can never be read again: the Keystore key is gone
     * ([KeyMissingException]), was invalidated, the ciphertext doesn't verify ([AEADBadTagException]), or the file
     * isn't in our format. An [UnrecoverableKeyException] counts only when nothing under it says the Keystore had a
     * temporary problem: it is permanent when it has no `KeyStoreException` cause, or (API 33+) when that cause's
     * `isTransientFailure()` is false. Everything else (a busy or restarting Keystore, `ProviderException`, I/O
     * errors, ...) is [Kind.TRANSIENT].
     */
    fun classify(error: Throwable, probe: KeystoreCauseProbe = AndroidKeystoreCauseProbe): Kind {
        var current: Throwable? = error
        var depth = 0
        while (current != null && depth++ < MAX_CAUSE_DEPTH) {
            if (current is KeyMissingException || current is CorruptKeyFileException ||
                current is KeyPermanentlyInvalidatedException || current is AEADBadTagException ||
                (current is UnrecoverableKeyException && isPermanent(current, probe))
            ) return Kind.UNUSABLE
            current = current.cause
        }
        return Kind.TRANSIENT
    }

    /** An `UnrecoverableKeyException` that may be wrapping a temporary Keystore error is not a lost key. */
    private fun isPermanent(error: UnrecoverableKeyException, probe: KeystoreCauseProbe): Boolean {
        var cause = error.cause
        var depth = 0
        while (cause != null && depth++ < MAX_CAUSE_DEPTH) {
            val transient = probe.transientFailure(cause)
            if (transient != null) return !transient
            cause = cause.cause
        }
        return true
    }

    /** How long to wait before retry number [attempt] (0 based) while the identity is unavailable. */
    fun retryDelayMillis(attempt: Int): Long = RETRY_STEPS_MILLIS.getOrElse(attempt) { RETRY_STEPS_MILLIS.last() }

    private val RETRY_STEPS_MILLIS = listOf(500L, 2_000L, 5_000L, 15_000L, 30_000L, 60_000L)
    private const val MAX_CAUSE_DEPTH = 8
}

/** What [IdentityStore.loadOrCreate] found. [replaced] says why a stored identity was thrown away, or is null. */
class IdentityLoad(val identity: Identity, val replaced: String?)

/**
 * Keeps the identity in two files: the PKCS#8 private key, encrypted by a [KeyWrapper], and the
 * certificate (public). Rules:
 * - A stored identity is replaced only when it is clearly lost or unusable: the files are incomplete,
 *   the key file is corrupt, the wrapping key is gone or invalid, or the certificate doesn't belong to the key.
 * - Any other failure throws [IdentityUnavailableException] and leaves the files untouched.
 * - Writes go key first, then certificate, each as write-to-temp, flush, rename. A crash between the two
 *   leaves a key without a certificate, which the next load treats as incomplete (it was never used).
 */
class IdentityStore(
    private val directory: File,
    private val wrapper: KeyWrapper,
    private val keystoreProbe: KeystoreCauseProbe = AndroidKeystoreCauseProbe,
    private val writeFile: (File, ByteArray) -> Unit = ::writeAtomically,
) {
    private val keyFile get() = File(directory, "identity.key")
    private val certFile get() = File(directory, "identity.crt")

    private sealed interface Loaded {
        class Found(val identity: Identity) : Loaded
        data object Absent : Loaded
        class Unusable(val reason: String) : Loaded
    }

    @Synchronized
    @Throws(IdentityUnavailableException::class)
    fun loadOrCreate(): IdentityLoad {
        val replaced = when (val loaded = load()) {
            is Loaded.Found -> return IdentityLoad(loaded.identity, null)
            Loaded.Absent -> null
            is Loaded.Unusable -> loaded.reason
        }
        val identity = Identity.generate()
        save(identity)
        return IdentityLoad(identity, replaced)
    }

    private fun load(): Loaded {
        val hasKey = keyFile.exists()
        val hasCert = certFile.exists()
        if (!hasKey && !hasCert) return Loaded.Absent
        if (!hasKey || !hasCert) return Loaded.Unusable("the identity files are incomplete (key file $hasKey, certificate $hasCert)")

        val wrapped: ByteArray
        val certificateDer: ByteArray
        try {
            wrapped = keyFile.readBytes()
            certificateDer = certFile.readBytes()
        } catch (e: IOException) {
            throw IdentityUnavailableException("the identity files can't be read: ${e.javaClass.simpleName}", e)
        }

        val pkcs8 = try {
            wrapper.decrypt(wrapped)
        } catch (e: Exception) {
            return when (IdentityFailure.classify(e, keystoreProbe)) {
                IdentityFailure.Kind.UNUSABLE -> Loaded.Unusable("the key can't be decrypted for good (${e.javaClass.simpleName})")
                IdentityFailure.Kind.TRANSIENT ->
                    throw IdentityUnavailableException("the key can't be decrypted right now: ${e.javaClass.simpleName}", e)
            }
        }
        val identity = try {
            Identity(Identity.privateKeyFromPkcs8(pkcs8), certificateDer)
        } catch (e: Exception) {
            return Loaded.Unusable("the stored key or certificate can't be parsed (${e.javaClass.simpleName})")
        }
        if (!keyMatchesCertificate(identity)) return Loaded.Unusable("the certificate doesn't match the private key")
        return Loaded.Found(identity)
    }

    private fun save(identity: Identity) {
        val encrypted = try {
            wrapper.encrypt(identity.privateKey.encoded)
        } catch (e: Exception) {
            throw IdentityUnavailableException("the key can't be encrypted right now: ${e.javaClass.simpleName}", e)
        }
        try {
            directory.mkdirs()
            writeFile(keyFile, encrypted)
            writeFile(certFile, identity.certificateDer)
        } catch (e: IOException) {
            throw IdentityUnavailableException("the identity can't be written: ${e.javaClass.simpleName}", e)
        }
    }

    private companion object {
        /** Proof of possession: a signature made with the private key must verify with the certificate's key. */
        fun keyMatchesCertificate(identity: Identity): Boolean = try {
            val message = "kin-identity-check".toByteArray()
            val signature = Signature.getInstance("SHA256withECDSA").run {
                initSign(identity.privateKey)
                update(message)
                sign()
            }
            Signature.getInstance("SHA256withECDSA").run {
                initVerify(identity.certificate.publicKey)
                update(message)
                verify(signature)
            }
        } catch (_: Exception) {
            false
        }
    }
}

/** Writes [bytes] to a temporary file next to [target], flushes it to disk, then renames it over [target]. */
internal fun writeAtomically(target: File, bytes: ByteArray) {
    val tmp = File(target.path + ".tmp")
    FileOutputStream(tmp).use {
        it.write(bytes)
        it.fd.sync()
    }
    Files.move(tmp.toPath(), target.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
}
