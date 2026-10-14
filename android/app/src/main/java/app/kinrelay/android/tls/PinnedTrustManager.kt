package app.kinrelay.android.tls

import android.annotation.SuppressLint
import app.kinrelay.android.identity.CertificateInfo
import app.kinrelay.android.identity.DeviceIds
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.X509TrustManager

class PinMismatchException(val expected: String, val actual: String) :
    CertificateException("the server's key is not the pinned device")

/**
 * Trusts exactly one server key: the one whose device ID (base64url SHA-256 of the SPKI) is
 * [pinnedDeviceId]. Hostname, issuer and dates are ignored on purpose (protocol.md, "Transport").
 * This is the only trust decision of the connection, so there is no fallback to the system store.
 */
@SuppressLint("CustomX509TrustManager") // the pin is the whole trust model (D4, D5): it never skips a check, it replaces the CA check
class PinnedTrustManager(private val pinnedDeviceId: String) : X509TrustManager {
    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        val leaf = chain?.firstOrNull() ?: throw CertificateException("the server sent no certificate")
        val actual = try {
            DeviceIds.deviceId(CertificateInfo.spkiOf(leaf.encoded))
        } catch (e: IllegalArgumentException) {
            throw CertificateException("the server certificate could not be read", e)
        }
        val equal = MessageDigest.isEqual(actual.toByteArray(Charsets.US_ASCII), pinnedDeviceId.toByteArray(Charsets.US_ASCII))
        if (!equal) throw PinMismatchException(pinnedDeviceId, actual)
    }

    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        throw CertificateException("this side is never a TLS server")
    }

    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
}
