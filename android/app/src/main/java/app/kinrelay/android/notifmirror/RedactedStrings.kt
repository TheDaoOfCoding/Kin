package app.kinrelay.android.notifmirror

/**
 * The text Android 15 and later puts in place of the content of a notification in which it found a one-time code, when the app that reads
 * it is an untrusted notification listener (design 4.9). It is the system string `redacted_notification_message` ("Sensitive notification
 * content hidden"), taken from the Android 37 SDK platform resources (`platforms/android-37.0/data/res`, every `values` folder's `strings.xml`: 86 locale files,
 * 81 different strings). Android localizes it with the phone's language, so a notification seen in another language needs its line here.
 * Compared after trimming and case-folding (`RedactionMatcher`). The newest Android versions may change the wording: add the new text, never remove one.
 */
internal object RedactedStrings {
    val ALL: List<String> = listOf(
        "Sensitiewe kennisgewinginhoud is versteek", // af
        "አደገኛ የማሳወቂያ ይዘት ተደብቋል", // am
        "تم إخفاء المحتوى الحساس في الإشعار", // ar
        "সংবেদনশীল জাননী লুকুওৱা হৈছে", // as
        "Həssas bildiriş kontenti gizlədildi", // az
        "Osetljiv sadržaj obaveštenja je skriven", // b+sr+Latn
        "Канфідэнцыяльнае змесціва ў апавяшчэннях схавана", // be
        "Деликатното съдържание в известието е скрито", // bg
        "সংবেদনশীল বিজ্ঞপ্তির কন্টেন্ট লুকানো আছে", // bn
        "Sakriven je osjetljiv sadržaj obavještenja", // bs
        "S'ha amagat contingut sensible de les notificacions", // ca
        "Obsah citlivých oznámení je skrytý", // cs
        "Følsomt indhold i notifikationen er skjult", // da
        "Vertrauliche Benachrichtigungsinhalte ausgeblendet", // de
        "Έγινε απόκρυψη της ειδοποίησης ευαίσθητου περιεχομένου", // el
        "Sensitive notification content hidden", // en-rAU, en-rCA, en-rGB, en-rIN, en
        "‎‏‎‎‎‎‎‏‎‏‏‏‎‎‎‎‎‎‏‎‎‏‎‎‎‎‏‏‏‏‏‎‏‏‎‏‎‏‎‎‎‏‏‎‏‎‎‎‏‏‎‏‏‎‏‎‎‏‎‏‏‎‏‏‏‎‎‎‎‏‎‎‏‏‎‎‏‏‏‏‎‏‎‏‏‎‎‏‎‏‎‎‎‎‎Sensitive notification content hidden‎‏‎‎‏‎", // en-rXC
        "Se ocultó contenido sensible de la notificación", // es-rUS
        "Contenido sensible de la notificación oculto", // es
        "Märguande delikaatne sisu peideti", // et
        "Jakinarazpenaren kontuzko edukia ezkutatu da", // eu
        "محتوای اعلان حساس پنهان شده است", // fa
        "Arkaluontoisen ilmoituksen sisältö piilotettu", // fi
        "Le contenu confidentiel de la notification est masqué", // fr-rCA
        "Le contenu sensible de la notification a été masqué", // fr
        "Contido confidencial da notificación oculto", // gl
        "સંવેદનશીલ માહિતીવાળા નોટિફિકેશનનું કન્ટેન્ટ છુપાવ્યું", // gu
        "संवेदनशील जानकारी वाली सूचना का कॉन्टेंट छिपा है", // hi
        "Skriven je osjetljiv sadržaj obavijesti", // hr
        "Bizalmas értesítéstartalom elrejtve", // hu
        "Ծանուցման զգայուն բովանդակությունը թաքցված է", // hy
        "Konten notifikasi sensitif disembunyikan", // in
        "Viðkvæmt tilkynningaefni falið", // is
        "Contenuti sensibili della notifica nascosti", // it
        "יש תוכן רגיש בהתראה שהוסתר", // iw
        "プライベートな通知内容は表示されません", // ja
        "სენსიტიური შეტყობინების კონტენტი დამალულია", // ka
        "Хабарландырудың құпия контенті жасырылған.", // kk
        "បានលាក់ខ្លឹមសារជូនដំណឹងដែលមានលក្ខណៈរសើប", // km
        "ಸೂಕ್ಷ್ಮ ನೋಟಿಫಿಕೇಶನ್ ಕಂಟೆಂಟ್ ಅನ್ನು ಮರೆಮಾಡಲಾಗಿದೆ", // kn
        "민감한 알림 콘텐츠 숨김", // ko
        "Купуя билдирменин мазмуну жашырылган", // ky
        "ເນື້ອຫາການແຈ້ງເຕືອນທີ່ລະອຽດອ່ອນເຊື່ອງຢູ່", // lo
        "Neskelbtinos informacijos pranešimo turinys paslėptas", // lt
        "Sensitīvs paziņojuma saturs ir paslēpts", // lv
        "Содржината на чувствителните известувања е скриена", // mk
        "സൂക്ഷ്‌മമായി കൈകാര്യം ചെയ്യേണ്ട അറിയിപ്പ് ഉള്ളടക്കം മറച്ചിരിക്കുന്നു", // ml
        "Эмзэг мэдэгдлийн контентыг нуусан", // mn
        "संवेदनशील नोटिफिकेशनचा आशय लपवलेला आहे", // mr
        "Kandungan pemberitahuan yang sensitif disembunyikan", // ms
        "သတိထားရမည့် အကြောင်းကြားချက်ပါ အချက်အလက်ကို ဖျောက်ထားသည်", // my
        "Sensitivt varselinnhold er skjult", // nb
        "संवेदनशील सूचनासम्बन्धी सामग्री लुकाइएको छ", // ne
        "Content van gevoelige meldingen verborgen", // nl
        "ସମ୍ୱେଦନଶୀଳ ବିଜ୍ଞପ୍ତି ବିଷୟବସ୍ତୁକୁ ଲୁଚାଯାଇଛି", // or
        "ਲੁਕੀ ਹੋਈ ਸੰਵੇਦਨਸ਼ੀਲ ਸੂਚਨਾ ਸਮੱਗਰੀ", // pa
        "Treść poufnego powiadomienia została ukryta", // pl
        "Conteúdo de notificação sensível oculto", // pt-rBR, pt
        "Conteúdo das notificações sensíveis ocultado", // pt-rPT
        "Conținutul sensibil din notificări a fost ascuns", // ro
        "Конфиденциальная информация в уведомлении скрыта", // ru
        "සංවේදී දැනුම්දීම් අන්තර්ගතය සැඟවී ඇත", // si
        "Obsah citlivého upozornenia je skrytý", // sk
        "Občutljiva vsebina obvestila je bila skrita", // sl
        "Përmbajtjet delikate të njoftimeve janë fshehur", // sq
        "Осетљив садржај обавештења је скривен", // sr
        "Känsligt aviseringsinnehåll dolt", // sv
        "Maudhui nyeti kwenye arifa yamefichwa", // sw
        "உணர்வுபூர்வமான அறிவிப்பு உள்ளடக்கம் மறைக்கப்பட்டது", // ta
        "గోప్యమైన నోటిఫికేషన్ కంటెంట్ దాచబడింది", // te
        "เนื้อหาการแจ้งเตือนที่ละเอียดอ่อนซ่อนอยู่", // th
        "Nakatago ang content ng sensitibong notification", // tl
        "Hassas bildirim içerikleri gizlendi", // tr
        "Чутливий вміст сповіщення приховано", // uk
        "حساس اطلاعی مواد چھپا ہوا ہے", // ur
        "Bildirishnomadagi maxfiy axborot berkitildi", // uz
        "Đã ẩn nội dung thông báo nhạy cảm", // vi
        "已隐藏敏感通知内容", // zh-rCN
        "已隱藏敏感通知內容", // zh-rHK
        "系統已隱藏含有私密資訊的通知內容", // zh-rTW
        "Okuqukethwe kwesaziso esizwelayo kufihliwe", // zu
    )
}
