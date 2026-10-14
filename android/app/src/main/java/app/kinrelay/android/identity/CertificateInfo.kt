package app.kinrelay.android.identity

/**
 * The parts of an X.509 certificate that matter to the protocol: the SPKI (for the device ID)
 * and the fields of the certificate profile. Parses DER strictly enough to read what this app
 * and the PC write; it is not a general certificate parser.
 */
class CertificateInfo private constructor(
    val version: Int,
    val serial: ByteArray,
    val innerSignatureAlgorithm: AlgorithmId,
    val outerSignatureAlgorithm: AlgorithmId,
    val notBeforeTag: Int,
    val notBefore: String,
    val notAfterTag: Int,
    val notAfter: String,
    val spki: ByteArray,
    val keyAlgorithmOid: String,
    val keyCurveOid: String?,
    val extensions: List<Extension>,
) {
    class AlgorithmId(val oid: String, val hasParameters: Boolean)

    class Extension(val oid: String, val critical: Boolean)

    private class Tlv(val tag: Int, val start: Int, val contentStart: Int, val end: Int)

    companion object {
        const val ECDSA_WITH_SHA256 = "1.2.840.10045.4.3.2"
        const val EC_PUBLIC_KEY = "1.2.840.10045.2.1"
        const val PRIME256V1 = "1.2.840.10045.3.1.7"

        fun parse(der: ByteArray): CertificateInfo {
            val cert = read(der, 0, der.size, Der.TAG_SEQUENCE)
            require(cert.end == der.size) { "trailing bytes after the certificate" }
            val tbs = read(der, cert.contentStart, cert.end, Der.TAG_SEQUENCE)
            val outerAlg = read(der, tbs.end, cert.end, Der.TAG_SEQUENCE)
            val signature = read(der, outerAlg.end, cert.end, Der.TAG_BIT_STRING)
            require(signature.end == cert.end) { "unexpected data after the signature" }

            var pos = tbs.contentStart
            var version = 0
            peekTag(der, pos, tbs.end)?.takeIf { it == Der.TAG_CONTEXT_0 }?.let {
                val wrapper = read(der, pos, tbs.end, Der.TAG_CONTEXT_0)
                val v = read(der, wrapper.contentStart, wrapper.end, Der.TAG_INTEGER)
                require(v.end - v.contentStart == 1) { "version too long" }
                version = der[v.contentStart].toInt()
                pos = wrapper.end
            }
            val serial = read(der, pos, tbs.end, Der.TAG_INTEGER)
            pos = serial.end
            val innerAlg = read(der, pos, tbs.end, Der.TAG_SEQUENCE)
            pos = innerAlg.end
            val issuer = read(der, pos, tbs.end, Der.TAG_SEQUENCE)
            val validity = read(der, issuer.end, tbs.end, Der.TAG_SEQUENCE)
            val notBefore = readAny(der, validity.contentStart, validity.end)
            val notAfter = readAny(der, notBefore.end, validity.end)
            val subject = read(der, validity.end, tbs.end, Der.TAG_SEQUENCE)
            val spki = read(der, subject.end, tbs.end, Der.TAG_SEQUENCE)
            pos = spki.end

            val spkiBytes = der.copyOfRange(spki.start, spki.end)
            val (keyOid, curve) = parseSpki(spkiBytes)

            val extensions = mutableListOf<Extension>()
            while (pos < tbs.end) {
                val item = readAny(der, pos, tbs.end)
                if (item.tag == 0xA3) {
                    val list = read(der, item.contentStart, item.end, Der.TAG_SEQUENCE)
                    var e = list.contentStart
                    while (e < list.end) {
                        val ext = read(der, e, list.end, Der.TAG_SEQUENCE)
                        val oidTlv = read(der, ext.contentStart, ext.end, Der.TAG_OID)
                        val oid = readOid(der, oidTlv)
                        var critical = false
                        if (peekTag(der, oidTlv.end, ext.end) == 0x01) {
                            val flag = readAny(der, oidTlv.end, ext.end)
                            critical = der[flag.contentStart] != 0.toByte()
                        }
                        extensions += Extension(oid, critical)
                        e = ext.end
                    }
                }
                pos = item.end
            }

            return CertificateInfo(
                version = version,
                serial = der.copyOfRange(serial.contentStart, serial.end),
                innerSignatureAlgorithm = algorithm(der, innerAlg),
                outerSignatureAlgorithm = algorithm(der, outerAlg),
                notBeforeTag = notBefore.tag,
                notBefore = String(der, notBefore.contentStart, notBefore.end - notBefore.contentStart, Charsets.US_ASCII),
                notAfterTag = notAfter.tag,
                notAfter = String(der, notAfter.contentStart, notAfter.end - notAfter.contentStart, Charsets.US_ASCII),
                spki = spkiBytes,
                keyAlgorithmOid = keyOid,
                keyCurveOid = curve,
                extensions = extensions,
            )
        }

        /** Key algorithm OID and, if present, the curve OID of a SubjectPublicKeyInfo. */
        fun parseSpki(spki: ByteArray): Pair<String, String?> {
            val outer = read(spki, 0, spki.size, Der.TAG_SEQUENCE)
            val algorithm = read(spki, outer.contentStart, outer.end, Der.TAG_SEQUENCE)
            val keyOid = read(spki, algorithm.contentStart, algorithm.end, Der.TAG_OID)
            val curve = if (keyOid.end < algorithm.end) {
                readOid(spki, read(spki, keyOid.end, algorithm.end, Der.TAG_OID))
            } else null
            return readOid(spki, keyOid) to curve
        }

        /** The SPKI exactly as it appears in the certificate. */
        fun spkiOf(certificateDer: ByteArray): ByteArray = parse(certificateDer).spki

        private fun algorithm(der: ByteArray, seq: Tlv): AlgorithmId {
            val oid = read(der, seq.contentStart, seq.end, Der.TAG_OID)
            return AlgorithmId(readOid(der, oid), oid.end < seq.end)
        }

        private fun peekTag(der: ByteArray, pos: Int, end: Int): Int? =
            if (pos < end) der[pos].toInt() and 0xFF else null

        private fun read(der: ByteArray, pos: Int, end: Int, tag: Int): Tlv {
            val tlv = readAny(der, pos, end)
            require(tlv.tag == tag) { "expected tag 0x${tag.toString(16)} at $pos, found 0x${tlv.tag.toString(16)}" }
            return tlv
        }

        private fun readAny(der: ByteArray, pos: Int, end: Int): Tlv {
            require(pos + 2 <= end) { "truncated DER at $pos" }
            val tag = der[pos].toInt() and 0xFF
            val first = der[pos + 1].toInt() and 0xFF
            var contentStart = pos + 2
            val length: Int
            if (first < 0x80) {
                length = first
            } else {
                val count = first and 0x7F
                require(count in 1..3 && contentStart + count <= end) { "bad DER length at $pos" }
                var value = 0
                repeat(count) { value = (value shl 8) or (der[contentStart + it].toInt() and 0xFF) }
                require(value >= 0x80) { "non-minimal DER length at $pos" }
                length = value
                contentStart += count
            }
            require(contentStart + length <= end) { "DER element at $pos overruns its parent" }
            return Tlv(tag, pos, contentStart, contentStart + length)
        }

        private fun readOid(der: ByteArray, tlv: Tlv): String {
            val arcs = mutableListOf<Long>()
            var value = 0L
            for (i in tlv.contentStart until tlv.end) {
                val b = der[i].toInt() and 0xFF
                value = (value shl 7) or (b and 0x7F).toLong()
                if (b and 0x80 == 0) {
                    if (arcs.isEmpty()) {
                        val first = minOf(value / 40, 2L)
                        arcs += first
                        arcs += value - first * 40
                    } else {
                        arcs += value
                    }
                    value = 0
                }
            }
            return arcs.joinToString(".")
        }
    }
}
