package app.kinrelay.android.tls

import app.kinrelay.android.identity.Identity
import java.net.Socket
import java.security.Principal
import java.security.PrivateKey
import java.security.cert.X509Certificate
import javax.net.ssl.SSLEngine
import javax.net.ssl.X509ExtendedKeyManager

/** Presents this install's certificate to the PC, which makes a client certificate mandatory. */
class ClientKeyManager(private val identity: Identity) : X509ExtendedKeyManager() {
    private fun alias(keyTypes: Array<out String>?): String? =
        if (keyTypes == null || keyTypes.any { it.startsWith("EC", ignoreCase = true) }) ALIAS else null

    override fun chooseClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, socket: Socket?) = alias(keyType)

    override fun chooseEngineClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, engine: SSLEngine?) = alias(keyType)

    override fun getClientAliases(keyType: String?, issuers: Array<out Principal>?): Array<String> = arrayOf(ALIAS)

    override fun getCertificateChain(alias: String?): Array<X509Certificate>? =
        if (alias == ALIAS) arrayOf(identity.certificate) else null

    override fun getPrivateKey(alias: String?): PrivateKey? = if (alias == ALIAS) identity.privateKey else null

    override fun getServerAliases(keyType: String?, issuers: Array<out Principal>?): Array<String>? = null

    override fun chooseServerAlias(keyType: String?, issuers: Array<out Principal>?, socket: Socket?): String? = null

    private companion object {
        const val ALIAS = "kin"
    }
}
