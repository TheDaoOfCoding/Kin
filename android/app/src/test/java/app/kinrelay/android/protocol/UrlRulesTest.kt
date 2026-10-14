package app.kinrelay.android.protocol

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UrlRulesTest {
    @Test
    fun oneWholeHttpOrHttpsUrlIsAUrl() {
        val yes = listOf(
            "http://example.com",
            "https://example.com",
            "HTTPS://EXAMPLE.COM/Path",
            "https://example.com/a?b=c#d",
            "https://user:pw@example.com:8443/x",
            "http://192.168.1.20:47821/",
            "http://[::1]:8080/x",
            "https://localhost",
            "https://例え.jp/パス",
            "https://example.com/a%20b",
            "  https://example.com  ",
            "\n\thttps://example.com\r\n",
            " https://example.com ",
            "https://example.com?q=1",
            "https://example.com#frag",
        )
        for (text in yes) assertTrue("'$text'", UrlRules.isUrl(text))
    }

    @Test
    fun everythingElseIsNot() {
        val no = listOf(
            "",
            "   ",
            "example.com",
            "www.example.com",
            "ftp://example.com",
            "file:///sdcard/x",
            "javascript:alert(1)",
            "data:text/html,<b>x</b>",
            "intent://x#Intent;scheme=http;end",
            "kinrelay://pair?v=1",
            "mailto:a@b.c",
            "http:/example.com",
            "http//example.com",
            "http://",
            "https://",
            "https:///path",
            "https://?q=1",
            "https://#x",
            "https://:80/",
            "https://user@/x",
            "https://[]/",
            "https://[::1",
            "https://host:80a/",
            "https://example.com and more",
            "see https://example.com",
            "https://example.com\nhttps://example.org",
            "https://exa mple.com",
            "https://example.com/a b",
            "https://example.com/\u0000",
            "https://example.com/ x",
            "https://example.com/\u0085x",
            "ht tps://example.com",
        )
        for (text in no) assertFalse("'${text.replace("\n", "\\n")}'", UrlRules.isUrl(text))
    }

    @Test
    fun httpUrlReturnsTheTrimmedUrlOrNull() {
        assertEquals("https://example.com/x", UrlRules.httpUrl("  https://example.com/x \n"))
        assertNull(UrlRules.httpUrl("javascript:alert(1)"))
        assertNull(UrlRules.httpUrl("https://example.com/x y"))
    }

    @Test
    fun onlyHttpAndHttpsMayBeOpened() {
        // The schemes an attacker would try through a notification action.
        for (scheme in listOf("javascript", "file", "content", "intent", "market", "tel", "sms", "ftp", "ws", "wss", "kinrelay", "android-app", "data")) {
            assertNull(scheme, UrlRules.httpUrl("$scheme://example.com/x"))
        }
        assertEquals("http://example.com", UrlRules.httpUrl("http://example.com"))
        assertEquals("https://example.com", UrlRules.httpUrl("https://example.com"))
    }
}
