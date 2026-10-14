package app.kinrelay.android.identity

import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption

/** Encrypts and decrypts the private key bytes. The real one uses the Android Keystore. */
interface KeyWrapper {
    fun encrypt(plain: ByteArray): ByteArray
    fun decrypt(wrapped: ByteArray): ByteArray
}

/**
 * Keeps the identity in two files: the certificate (public) and the PKCS#8 private key,
 * encrypted by a [KeyWrapper].
 */
class IdentityStore(
    private val directory: File,
    private val wrapper: KeyWrapper,
    private val onRegenerated: (String) -> Unit = {},
) {
    private val keyFile get() = File(directory, "identity.key")
    private val certFile get() = File(directory, "identity.crt")

    @Synchronized
    fun loadOrCreate(): Identity {
        load()?.let { return it }
        val identity = Identity.generate()
        save(identity)
        return identity
    }

    private fun load(): Identity? {
        if (!keyFile.exists() || !certFile.exists()) return null
        return try {
            val pkcs8 = wrapper.decrypt(keyFile.readBytes())
            Identity(Identity.privateKeyFromPkcs8(pkcs8), certFile.readBytes())
        } catch (e: Exception) {
            // The wrapping key is gone (for example a restored backup): the old identity can't be used.
            onRegenerated("identity could not be read (${e.javaClass.simpleName}); creating a new one")
            null
        }
    }

    private fun save(identity: Identity) {
        directory.mkdirs()
        writeAtomically(certFile, identity.certificateDer)
        writeAtomically(keyFile, wrapper.encrypt(identity.privateKey.encoded))
    }

    private fun writeAtomically(target: File, bytes: ByteArray) {
        val tmp = File(target.path + ".tmp")
        tmp.writeBytes(bytes)
        Files.move(tmp.toPath(), target.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
    }
}
