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
            "https://example.com/a@b",
            "https://example.com/?email=a@b.c#x@y",
            "https://example.com/a\\b",
            "https://example.com/a%20b%41",
            "https://xn--bcher-kva.example/",
            "https://ｇｏｏｇｌｅ.com/",
            "http://0x7f.1/",
            "https://256.1.1.1/",
            "https://[abc]/",
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
            // changelog 55: no user information, no backslash and no percent-encoding in the authority, and no [ ] < > ^ | in a host
            "https://user:pw@example.com:8443/x",
            "https://google.com@evil.example/",
            "https://google.com:443@evil.example/",
            "https://me@[::1]:9/",
            "https://evil.example\\@google.com",
            "https://evil.example\\/x",
            "https://example.com\\",
            "https://\\example.com/",
            "https://goog%6Ce.com/",
            "https://%67oogle.com/",
            "http://[fe80::1%25eth0]/",
            "https://example.com:80%30/",
            "https://exa[mple.com/",
            "https://exa]mple.com/",
            "https://exa<mple.com/",
            "https://exa>mple.com/",
            "https://exa^mple.com/",
            "https://exa|mple.com/",
            "https://[::1]x/",
            "https://[]/",
            "https://[::1",
            "https://host:80a/",
            "https://example.com and more",
            "see https://example.com",
            "https://example.com\nhttps://example.org",
            "https://exa mple.com",
            "https://example.com/a b",
            "https://example.com/\u0000",
            "https://example.com/\u2028x",
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
    fun theTightenedRuleRejectsTheAuthorityTricksButNotTheirLookAlikesAfterTheAuthority() {
        // Android parses `https://evil.example\@google.com` as google.com, a browser as evil.example: the URL is refused, not shown with a mark.
        assertNull(UrlRules.httpUrl("https://evil.example\\@google.com"))
        assertNull(UrlRules.httpUrl("https://google.com@evil.example/"))
        assertEquals("https://example.com/a@b?x=\\@evil.com#%41", UrlRules.httpUrl("https://example.com/a@b?x=\\@evil.com#%41"))
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
