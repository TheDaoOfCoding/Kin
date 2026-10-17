package app.kinrelay.android.notifmirror

/**
 * The keyword table of the one-time-code detector (design 4.10). A message with a number and none of these words is never a code. Extend
 * it through vectors (`otpDetect[]`), never by removing a negative. Every entry is compared lower-cased (`Locale.ROOT`).
 */
object OtpKeywords {
    val ENGLISH = listOf(
        "code", "otp", "passcode", "pass code", "verification", "verify", "one-time", "one time", "security code", "login code",
        "sign-in code", "sign in code", "2fa", "two-factor", "two factor", "pin", "confirmation code",
    )
    val SPANISH_PORTUGUESE = listOf(
        "código", "codigo", "verificación", "verificacion", "verificação", "clave", "contraseña de un solo uso", "senha",
        "confirmación", "confirmação",
    )
    val FRENCH = listOf("code", "vérification", "verification", "confirmation", "mot de passe à usage unique")
    val GERMAN = listOf("code", "bestätigungscode", "verifizierung", "sicherheitscode", "einmalpasswort", "tan")
    val ITALIAN_DUTCH = listOf("codice", "verifica", "conferma", "password monouso", "verificatie", "verificatiecode", "bevestigingscode")
    val NORDIC_POLISH = listOf("kod", "kode", "bekräftelsekod", "verifieringskod", "engangskode", "weryfikacyjny", "jednorazowy")
    val TURKISH_RUSSIAN_UKRAINIAN = listOf(
        "kod", "doğrulama", "onay kodu", "tek kullanımlık", "код", "подтверждения", "проверочный", "одноразовый", "підтвердження",
    )
    val ARABIC_PERSIAN_HINDI = listOf("رمز", "رمز التحقق", "كلمة مرور لمرة واحدة", "کد", "کد تایید", "कोड", "ओटीपी", "सत्यापन")
    val INDONESIAN_VIETNAMESE_THAI = listOf("kode", "verifikasi", "sandi sekali pakai", "mã", "xác minh", "mã xác thực", "รหัส", "ยืนยัน")
    val JAPANESE_KOREAN = listOf("認証コード", "確認コード", "認証番号", "確認番号", "ワンタイム", "コード", "인증번호", "인증 코드", "확인 코드", "일회용")
    val CHINESE = listOf("验证码", "校验码", "动态码", "确认码", "驗證碼", "校驗碼", "動態密碼")

    /** These are only ever matched as whole words, never inside another word. */
    val ALWAYS_WHOLE_WORD = setOf("otp", "2fa", "pin")

    val ALL: List<String> = (ENGLISH + SPANISH_PORTUGUESE + FRENCH + GERMAN + ITALIAN_DUTCH + NORDIC_POLISH + TURKISH_RUSSIAN_UKRAINIAN +
        ARABIC_PERSIAN_HINDI + INDONESIAN_VIETNAMESE_THAI + JAPANESE_KOREAN + CHINESE).distinct()

    /** Words that make a code-shaped token a coupon, not a code (design 4.10, "Negative examples"). */
    val PROMO = listOf("save", "off", "discount", "promo", "coupon", "sale")
}
