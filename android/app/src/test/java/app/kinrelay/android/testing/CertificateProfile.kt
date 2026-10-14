package app.kinrelay.android.testing

import app.kinrelay.android.identity.CertificateInfo

/** The certificate profile of protocol.md, "Transport", as a list of violations (empty means it fits). */
object CertificateProfile {
    fun violations(der: ByteArray): List<String> {
        val v = mutableListOf<String>()
        val info = try {
            CertificateInfo.parse(der)
        } catch (e: IllegalArgumentException) {
            return listOf("not a parseable certificate: ${e.message}")
        }
        if (info.version != 2) v += "version is not v3 (field ${info.version})"
        val s = info.serial
        if (s.isEmpty() || s.size > 20) v += "serial is ${s.size} bytes"
        if (s.isNotEmpty() && s[0].toInt() and 0x80 != 0) v += "serial is negative"
        if (s.all { it == 0.toByte() }) v += "serial is zero"
        if (s.size > 1 && s[0] == 0.toByte() && s[1].toInt() and 0x80 == 0) v += "serial is not minimal DER"
        for ((label, alg) in listOf("inner" to info.innerSignatureAlgorithm, "outer" to info.outerSignatureAlgorithm)) {
            if (alg.oid != CertificateInfo.ECDSA_WITH_SHA256) v += "$label signature algorithm is ${alg.oid}"
            if (alg.hasParameters) v += "$label signature algorithm has parameters"
        }
        for ((tag, text) in listOf(info.notBeforeTag to info.notBefore, info.notAfterTag to info.notAfter)) {
            when (tag) {
                0x17 -> if (!Regex("[0-9]{12}Z").matches(text)) v += "UTCTime $text is malformed"
                0x18 -> when {
                    !Regex("[0-9]{14}Z").matches(text) -> v += "GeneralizedTime $text is malformed"
                    text.take(4).toInt() < 2050 -> v += "GeneralizedTime $text is before 2050"
                }
                else -> v += "a validity bound is not a time"
            }
        }
        if (info.spki.size != 91) v += "SPKI is ${info.spki.size} bytes"
        if (info.keyAlgorithmOid != CertificateInfo.EC_PUBLIC_KEY) v += "key type is ${info.keyAlgorithmOid}"
        if (info.keyCurveOid != CertificateInfo.PRIME256V1) v += "curve is ${info.keyCurveOid}"
        if (info.spki.size == 91 && !(info.spki[25] == 0.toByte() && info.spki[26] == 4.toByte())) v += "not an uncompressed point"
        info.extensions.filter { it.critical }.forEach { v += "critical extension ${it.oid}" }
        return v
    }
}
