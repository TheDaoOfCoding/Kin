package app.kinrelay.android.log

import app.kinrelay.android.identity.KeyWrapper
import java.io.File
import java.security.SecureRandom
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * What a log line may say about a piece of text (a clipboard text, a shared text, a file name): its **length**, and a short **keyed**
 * hash, never the text. The hash is HMAC-SHA256 with a random key of this install, cut to 4 bytes. An unkeyed hash of a short text
 * can be guessed by hashing candidates (a PIN, a name); with a key that never leaves the phone (it is stored wrapped by the Android
 * Keystore, and is never exported) the hash only tells two lines of the same log that they are about the same text.
 *
 * Until the key is loaded ([install]) only the length is logged. A sensitive text is not fingerprinted at all (the callers log nothing).
 */
object LogFingerprint {
    @Volatile private var key: ByteArray? = null

    /** Sets the key of this install (null: length only). */
    fun install(key: ByteArray?) {
        this.key = key?.copyOf()
    }

    /** `len=<code points> sha=<8 hex>`, or `len=<code points>` while there is no key. */
    fun of(text: String, hashed: String = text): String {
        val length = text.codePointCount(0, text.length)
        val k = key ?: return "len=$length"
        return "len=$length sha=${mac(k, hashed)}"
    }

    /** The first 4 bytes of HMAC-SHA256([key], [text] as UTF-8), as 8 hex characters. */
    fun mac(key: ByteArray, text: String): String {
        val hmac = Mac.getInstance("HmacSHA256").apply { init(SecretKeySpec(key, "HmacSHA256")) }
        return hmac.doFinal(text.toByteArray(Charsets.UTF_8)).take(4).joinToString("") { "%02x".format(it) }
    }
}

/**
 * The random key of [LogFingerprint], kept in [file] wrapped by [wrapper] (the Android Keystore in the app). It is never written in clear
 * and never exported. A key that can't be read (the Keystore key is gone) is replaced by a new one: the hashes in older log lines then
 * stop matching, which costs nothing.
 */
class LogKeyStore(
    private val file: File,
    private val wrapper: KeyWrapper,
    private val random: SecureRandom = SecureRandom(),
) {
    fun loadOrCreate(): ByteArray {
        if (file.isFile) {
            val stored = runCatching { wrapper.decrypt(file.readBytes()) }.getOrNull()
            if (stored != null && stored.size == KEY_BYTES) return stored
        }
        val fresh = ByteArray(KEY_BYTES).also(random::nextBytes)
        val wrapped = wrapper.encrypt(fresh)
        file.parentFile?.mkdirs()
        val temporary = File(file.parentFile, file.name + ".tmp")
        temporary.writeBytes(wrapped)
        if (!temporary.renameTo(file)) {
            file.delete()
            check(temporary.renameTo(file)) { "can't store the log key" }
        }
        return fresh
    }

    private companion object {
        const val KEY_BYTES = 32
    }
}
