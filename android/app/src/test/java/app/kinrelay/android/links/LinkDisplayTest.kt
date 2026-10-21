package app.kinrelay.android.links

import androidx.core.app.NotificationCompat
import app.kinrelay.android.notify.NotificationLedger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What a link notification says (2B.1): the real address is always next to the title and is worked out from the URL, so a title (or a tricky URL)
 * can't pass for another address; the lock screen hides the title and the address by default; links are grouped. The address vectors
 * (`display[]` of `protocol/links.json`) are in `LinkVectorsTest`; this class has the cases around them. Internationalised hosts go through
 * [Icu4jUts46] here (`android.icu` is a stub on the JVM); the instrumented test runs them on the device's own ICU.
 */
class LinkDisplayTest {
    private fun accept(url: String, title: String? = null) = LinkRules.accept(url, title)!!

    private fun addressOf(url: String) = LinkDomain.of(url, Icu4jUts46)!!

    private fun domainOf(url: String) = addressOf(url).display

    // --- the address ---

    @Test
    fun theAddressIsTheHostInLowerCase() {
        assertEquals("www.youtube.com", domainOf("https://WWW.YouTube.com/watch?v=dQw4w9WgXcQ&t=83s"))
        assertEquals("example.com", domainOf("http://example.com"))
        assertEquals("example.com", domainOf("https://example.com?x=1#y"))
    }

    @Test
    fun aNonDefaultPortIsShownAndTheDefaultOneIsNot() {
        assertEquals("example.com:8443", domainOf("https://example.com:8443/a"))
        assertEquals("example.com", domainOf("https://example.com:443/a"))
        assertEquals("example.com", domainOf("http://example.com:80/"))
        assertEquals("example.com:8080", domainOf("http://example.com:08080/"))
        assertEquals("[::1]:8080", domainOf("https://[::1]:8080/x"))
    }

    @Test
    fun aLoginPartABackslashOrAnEscapeInTheAuthorityIsRefusedNotShownWithAMark() {
        // B1 (2B.1 review): Android reads `https://evil.example\@google.com` as google.com, a browser as evil.example. The URL is rejected at the
        // acceptance check, so neither the notification nor a tap ever sees it.
        for (bad in listOf(
            "https://evil.example\\@google.com/", "https://evil.example\\/x", "https://example.com\\", "https://\\example.com/",
            "https://accounts.google.com@evil.example/signin", "https://google.com:443@evil.example/", "https://user:pw@example.com:8443/a", "https://me@[::1]:9/",
            "https://goog%6Ce.com/", "http://[fe80::1%25eth0]/",
        )) {
            assertNull("refused: $bad", LinkRules.accept(bad, "Sign in"))
            assertNull("refused: $bad", LinkRules.acceptUrl(bad))
            assertNull("no address for $bad", LinkDomain.of(bad, Icu4jUts46))
        }
        // After the authority a backslash, an @ or a % is nothing special: the host is unchanged and nothing warns.
        val path = addressOf("https://example.com/a\\b?x=\\@evil.com#y@z%41")
        assertEquals("example.com", path.display)
        assertFalse(path.warn)
    }

    @Test
    fun anInternationalNameIsShownInItsAsciiFormAndWarns() {
        // Cyrillic "а" in "аpple.com" looks like the Latin one: it is shown as punycode and marked.
        val address = addressOf("https://\u0430pple.com/")
        assertEquals("xn--pple-43d.com", address.display)
        assertTrue(address.warn)
        assertEquals("xn--bcher-kva.example", domainOf("https://b\u00fccher.example/"))
        assertEquals("UTS 46 non-transitional, not IDNA 2003 (which says fass.de)", "xn--fa-hia.de", domainOf("https://fa\u00df.de/"))
    }

    @Test
    fun ipAddressesAreShownTheWayABrowserReadsThemAndWarn() {
        for ((url, shown) in listOf(
            "https://0x7f.1/" to "127.0.0.1", "https://2130706433/" to "127.0.0.1", "http://192.168.1.1:8080/" to "192.168.1.1:8080",
            "https://[::ffff:127.0.0.1]/" to "[::ffff:7f00:1]", "https://[2001:DB8:0:0:0:0:0:1]:8080/" to "[2001:db8::1]:8080",
        )) {
            val address = addressOf(url)
            assertEquals(url, shown, address.display)
            assertTrue("an IP address always warns: $url", address.warn)
        }
    }

    @Test
    fun aHostABrowserRefusesIsShownAsWrittenWithAWarningAndNoPort() {
        for ((url, shown) in listOf("https://1.2.3.4.5/" to "1.2.3.4.5", "https://256.1.1.1:8443/" to "256.1.1.1", "https://[abc]/" to "[abc]", "https://xn--a.com/" to "xn--a.com")) {
            val address = addressOf(url)
            assertEquals(url, shown, address.display)
            assertTrue(address.warn)
        }
        // Every character outside U+0021 to U+007E is U+FFFD: a bidi control never reaches the screen.
        val bidi = addressOf("https://exam\u202eple.com:8443/")
        assertEquals("exam\uFFFDple.com", bidi.display)
        assertTrue(bidi.warn)
        assertEquals("\uFFFD.com", addressOf("https://\u0378.com/").display)
    }

    @Test
    fun aPlainAsciiAddressNeverWarns() {
        for (url in listOf("https://example.com/", "https://paypa1.com/", "http://localhost:3000/", "https://example.com./", "http://intranet/x", "https://example.com:80/")) {
            assertFalse(url, addressOf(url).warn)
        }
    }

    @Test
    fun theUrlIsNeverRewrittenOnItsWayToTheOpener() {
        val youtube = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=83s&list=PLabc123#frag"
        assertEquals(youtube, accept(youtube).url)
        assertEquals("https://youtu.be/dQw4w9WgXcQ?t=1h2m3s", accept("https://youtu.be/dQw4w9WgXcQ?t=1h2m3s").url)
        assertEquals("HTTPS://Example.COM:443/A%20B", accept("HTTPS://Example.COM:443/A%20B").url)
    }

    // --- what is accepted ---

    @Test
    fun onlyHttpAndHttpsAreAcceptedAndNothingIsTrimmed() {
        for (bad in listOf("javascript:alert(1)", "file:///C:/Windows/notepad.exe", "intent://x#Intent;end", "data:text/html,hi", "ftp://example.com/", "kinrelay://pair?v=1", "mailto:a@b.c", "", " https://example.com/", "https://example.com/ ", "https://")) {
            assertNull("rejected: '$bad'", LinkRules.accept(bad, "title"))
            assertNull("rejected: '$bad'", LinkRules.acceptUrl(bad))
        }
        assertNotNull(LinkRules.accept("https://example.com/", null))
        assertNotNull(LinkRules.accept("http://192.168.1.5:8080/path", null))
    }

    // --- the notification's words ---

    @Test
    fun theRealAddressIsAlwaysThereAndATitleCanNotChangeIt() {
        val link = accept("https://evil.example/login", "https://accounts.google.com - Sign in")
        val content = LinkNotificationContent.of("Desk PC", link, "Open in browser", lockScreenPreviews = true)
        assertEquals("https://accounts.google.com - Sign in", content.title)
        assertEquals("the second line is the address of the URL, not of the title", "evil.example", content.addressLine)
        assertTrue("and so is the expanded text, first", content.bigText.startsWith("evil.example\n"))
    }

    @Test
    fun aLinkWithoutATitleShowsTheAddressAsTheTitleAndSaysWhereItCameFrom() {
        val content = LinkNotificationContent.of("Desk PC", accept("https://example.com/a"), "Open in browser", lockScreenPreviews = false)
        assertEquals("example.com", content.title)
        assertEquals("Link from Desk PC", content.addressLine)
    }

    @Test
    fun aWarnedAddressHasTheWarningSignInFrontOfIt() {
        // An address in another spelling (here an IP in hex) is shown the way a browser reads it, with the sign; ASCII hosts have none.
        val ip = LinkNotificationContent.of("Desk PC", accept("http://0x7f.1/login", "Google"), "Open in browser", lockScreenPreviews = true)
        assertEquals("\u26a0 127.0.0.1", ip.addressLine)
        assertTrue(ip.bigText.startsWith("\u26a0 127.0.0.1\n"))
        val plain = LinkNotificationContent.of("Desk PC", accept("https://example.com/", "Example"), "Open in browser", lockScreenPreviews = true)
        assertEquals("example.com", plain.addressLine)
    }

    @Test
    fun theSubTextSaysWhatATapDoes() {
        val link = accept("https://www.youtube.com/watch?v=abc", "A video")
        assertEquals("Open in YouTube", LinkNotificationContent.of("Desk PC", link, LinkOpenPlan.label(listOf(HandlerApp("com.google.android.youtube", "YouTube"))), false).subText)
        assertEquals("Open link", LinkNotificationContent.of("Desk PC", link, LinkOpenPlan.label(emptyList()), false).subText)
    }

    @Test
    fun theExpandedTextShowsTheUrlShortenedAndWithoutBidiControls() {
        val long = "https://example.com/" + "a".repeat(400)
        val content = LinkNotificationContent.of("Desk PC", accept(long, "T"), "Open in browser", false)
        val lines = content.bigText.lines()
        assertEquals("example.com", lines[0])
        assertTrue(lines[1].endsWith("…"))
        assertEquals(LinkNotificationContent.URL_SHOWN_CHARACTERS + 1, lines[1].length)
        assertEquals("From Desk PC", lines[2])
        assertEquals("a\uFFFDb", LinkRules.displayUrl("a\u202eb", 50))
    }

    // --- the lock screen ---

    @Test
    fun theLockScreenHidesTheTitleAndTheAddressUnlessTheUserAllowsIt() {
        val link = accept("https://example.com/secret-thing", "Private page")
        val hidden = LinkNotificationContent.of("Desk PC", link, "Open in browser", lockScreenPreviews = false)
        assertEquals("the default: private, the public version is a bare line", NotificationCompat.VISIBILITY_PRIVATE, hidden.visibility)
        assertEquals("Link from Desk PC", hidden.publicTitle)
        assertFalse(hidden.publicTitle.contains("example.com"))
        assertFalse(hidden.publicTitle.contains("Private page"))
        val shown = LinkNotificationContent.of("Desk PC", link, "Open in browser", lockScreenPreviews = true)
        assertEquals(NotificationCompat.VISIBILITY_PUBLIC, shown.visibility)
    }

    // --- grouping ---

    @Test
    fun severalLinksInARowAreOneGroupWithASummaryAndAtMostTenStayUp() {
        assertFalse("one link: no summary", LinkNotificationContent.hasSummary(1))
        assertTrue("two or more: a summary", LinkNotificationContent.hasSummary(2))
        assertEquals("3 links from Desk PC", LinkNotificationContent.summaryTitle(3, "Desk PC"))
        val ledger = NotificationLedger(LinkNotifications.MAX_LINKS)
        var pushedOut = emptyList<String>()
        for (n in 1..12) pushedOut = ledger.claim("m$n", "site$n.example")
        assertEquals(listOf("m2"), pushedOut)
        assertEquals("the oldest two went", (3..12).map { "m$it" }, ledger.keys)
        assertEquals("the summary lines are the addresses, never the titles", "site12.example", ledger.titles.last())
    }
}
