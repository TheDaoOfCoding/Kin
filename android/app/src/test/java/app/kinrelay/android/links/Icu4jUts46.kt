package app.kinrelay.android.links

import com.ibm.icu.text.IDNA

/**
 * [Uts46] on ICU4J for the JVM unit tests: `android.icu` is only a stub there. Same ICU API, same options and the same ignored errors as
 * [AndroidUts46], which the instrumented `LinkDisplayVectorsInstrumentedTest` runs on a real device (API 29 and API 37 differ in ICU version).
 */
object Icu4jUts46 : Uts46 {
    private val idna = IDNA.getUTS46Instance(IDNA.NONTRANSITIONAL_TO_ASCII or IDNA.CHECK_BIDI or IDNA.CHECK_CONTEXTJ)

    private val ignored = setOf(
        IDNA.Error.EMPTY_LABEL, IDNA.Error.LABEL_TOO_LONG, IDNA.Error.DOMAIN_NAME_TOO_LONG,
        IDNA.Error.LEADING_HYPHEN, IDNA.Error.TRAILING_HYPHEN, IDNA.Error.HYPHEN_3_4,
    )

    override fun toAscii(host: String): String? {
        val info = IDNA.Info()
        val out = StringBuilder()
        idna.nameToASCII(host, out, info)
        if (info.errors.any { it !in ignored }) return null
        return out.toString()
    }
}
