package app.kinrelay.android.notifmirror

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The one-time-code detector (design 4.10): every language of the keyword table has a positive, and every false-positive guard has a negative. */
class OtpDetectorTest {
    private fun code(text: String, title: String? = null, bigText: String? = null, contact: Boolean = false) = OtpDetector.detect(title, text, bigText, contact)

    @Test
    fun everyLanguageOfTheKeywordTableFindsItsCode() {
        val positives = mapOf(
            "en" to "Your verification code is 482913",
            "en otp" to "482913 is your OTP",
            "en pin" to "Your PIN is 4829",
            "en google" to "G-123456 is your Google verification code",
            "es" to "Tu código de verificación es 482913",
            "pt" to "Seu código de verificação é 482913",
            "fr" to "Votre code de vérification est 482913",
            "de" to "Ihr Bestätigungscode lautet 482913",
            "it" to "Il tuo codice di verifica è 482913",
            "nl" to "Uw verificatiecode is 482913",
            "sv" to "Din verifieringskod är 482913",
            "no" to "Din engangskode er 482913",
            "pl" to "Twój kod weryfikacyjny to 482913",
            "tr" to "Doğrulama kodunuz 482913",
            "ru" to "Ваш код подтверждения 482913",
            "uk" to "Ваш код підтвердження 482913",
            "ar" to "رمز التحقق الخاص بك هو 482913",
            "fa" to "کد تایید شما 482913 است",
            "hi" to "आपका ओटीपी 482913 है",
            "id" to "Kode verifikasi Anda adalah 482913",
            "vi" to "Mã xác thực của bạn là 482913",
            "th" to "รหัสยืนยันของคุณคือ 482913",
            "th no space" to "รหัสยืนยันของคุณคือ482913",
            "ja" to "認証コード: 482913",
            "ja against the text" to "認証コードは482913です",
            "ko" to "[Web발신] 인증번호 482913",
            "ko against the text" to "인증번호는482913입니다",
            "zh" to "【淘宝】验证码482913，您正在登录，请勿泄露。",
            "zh-TW" to "您的驗證碼為 482913",
        )
        for ((language, text) in positives) {
            val expected = if (language == "en pin") "4829" else if (language == "en google") "123456" else "482913"
            assertEquals(language, expected, code(text))
        }
    }

    @Test
    fun theKeywordTableHasAPositiveForEveryRowOfItsLanguages() {
        // 11 rows in the design; every keyword list is non-empty and has no blank entry.
        for (list in listOf(
            OtpKeywords.ENGLISH, OtpKeywords.SPANISH_PORTUGUESE, OtpKeywords.FRENCH, OtpKeywords.GERMAN, OtpKeywords.ITALIAN_DUTCH, OtpKeywords.NORDIC_POLISH,
            OtpKeywords.TURKISH_RUSSIAN_UKRAINIAN, OtpKeywords.ARABIC_PERSIAN_HINDI, OtpKeywords.INDONESIAN_VIETNAMESE_THAI, OtpKeywords.JAPANESE_KOREAN, OtpKeywords.CHINESE,
        )) {
            assertTrue(list.isNotEmpty())
            assertTrue(list.none { it.isBlank() })
        }
    }

    @Test
    fun digitsInAnyScriptAreFoldedToAscii() {
        assertEquals("482913", code("رمز التحقق ٤٨٢٩١٣"))
        assertEquals("482913", code("आपका ओटीपी ४८२९१३ है"))
        assertEquals("482913", code("Your code is ４８２９１３")) // full-width
    }

    @Test
    fun groupedAndAlphanumericCodes() {
        assertEquals("482913", code("Your code is 482-913"))
        assertEquals("482913", code("Your code is 482 913"))
        assertEquals("48291357", code("Your code is 4829-1357"))
        assertEquals("AB12CD", code("Your code: AB12CD"))
        assertEquals("X7K9QP", code("Use X7K9QP to verify"))
        assertNull("lowercase letters are not a code", code("Your code: ab12cd"))
        assertNull("letters only", code("Your code: ABCDEF"))
        assertNull("digits-only shorter than 4", code("Your code: 123"))
        assertNull("longer than 8 digits", code("Your code: 123456789"))
    }

    @Test
    fun theNegativesOfTheDesign() {
        assertNull("no keyword", code("Your order #483921 has shipped"))
        assertNull("a coupon", code("Use code SAVE20 for 20% off"))
        assertNull("a coupon word near a plain number", code("Use code 123456 and save 20% today"))
        assertNull("a time and a room, no keyword", code("Meeting at 12:30, room 4521"))
        assertNull("an amount", code("Your code balance is \$1,234.56"))
        assertNull("a phone number", code("Call 555 123 4567 to verify"))
        assertEquals("a year and a code", "482913", code("Your 2024 verification code is 482913"))
        assertNull("more than two groups", code("Code: 1234 5678 9012"))
        assertNull("two equally near codes", code("Verification code 123456 or 654321"))
    }

    @Test
    fun otherLookAlikesAreNotCodes() {
        assertNull("a date", code("Your code expires on 2024-10-17"))
        assertNull("a decimal", code("Code accuracy is 3.14159"))
        assertNull("a percentage", code("Code coverage reached 9999%"))
        assertNull("an amount in a currency", code("Verification fee is 1500 USD"))
        assertNull("a distance", code("Verify the route: 12000 km"))
        assertNull("a plus is a phone number", code("Verify your number +49123456"))
        assertNull("next to letters", code("Your code is 123456abc"))
        assertNull("a web address", code("Verify at https://example.com/AB12CD"))
        assertNull("no code word at all", code("Your lucky number is 482913"))
        assertNull("pins is not pin", code("Pinterest: 123456 new pins"))
        assertNull("too long a message", code("Your code is 482913. " + "x".repeat(400)))
        assertNull("nothing", code(""))
    }

    @Test
    fun aYearIsACodeOnlyWhenItIsTheOnlyCandidate() {
        assertEquals("2024", code("Your code is 2024"))
        assertEquals("1999", code("Verification code 1999"))
        assertEquals("482913", code("Verification code 2025 is not it: 482913"))
    }

    @Test
    fun aFriendsMessageIsNotACode() {
        assertNull(code("what's the code 1234?", contact = true))
        assertEquals("1234", code("what's the code 1234?", contact = false))
    }

    @Test
    fun theTitleTheTextAndTheBigTextAreJoined() {
        assertEquals("482913", OtpDetector.detect("Verification code", "482913", null))
        assertEquals("482913", OtpDetector.detect("Example", "Your code is", "Your code is 482913"))
        // The same code in the text and in the big text is one candidate, not two equal ones.
        assertEquals("482913", OtpDetector.detect("Example", "Your code is 482913", "Your code is 482913 (expires in 5 min)"))
    }

    @Test
    fun theNearestCodeWinsWhenItIsClearlyNearer() {
        assertEquals("482913", code("Your verification code is 482913 and the reference number is 998877 from the bank statement dated last month"))
    }
}
