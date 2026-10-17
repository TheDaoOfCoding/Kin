package app.kinrelay.android.links

import android.os.Build
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** What a tap on a link does (2B.1): the app that owns the link first, the default browser as the fallback, on Android 10 and on 11 and later. */
class LinkOpenPlanTest {
    private val youtube = HandlerApp("com.google.android.youtube", "YouTube", isDefault = true)
    private val other = HandlerApp("org.example.player", "Player")
    private val android10 = Build.VERSION_CODES.Q
    private val android11 = Build.VERSION_CODES.R
    private val android17 = 37

    @Test
    fun onAndroid11AndLaterTheNonBrowserFlagIsTriedFirstWhateverWeFound() {
        for (sdk in listOf(android11, 34, android17)) {
            assertEquals(listOf(OpenAttempt.RequireNonBrowser, OpenAttempt.Default), LinkOpenPlan.attempts(sdk, emptyList()))
            assertEquals(listOf(OpenAttempt.RequireNonBrowser, OpenAttempt.Default), LinkOpenPlan.attempts(sdk, listOf(youtube)))
        }
    }

    @Test
    fun onAndroid10TheResolveCheckPicksTheAppAndTheBrowserIsTheFallback() {
        assertEquals(listOf(OpenAttempt.InPackage("com.google.android.youtube"), OpenAttempt.Default), LinkOpenPlan.attempts(android10, listOf(youtube, other)))
        assertEquals("no app owns it: the default browser", listOf(OpenAttempt.Default), LinkOpenPlan.attempts(android10, emptyList()))
    }

    @Test
    fun theLabelNamesTheAppOrSaysBrowser() {
        assertEquals("Open in YouTube", LinkOpenPlan.label(listOf(youtube, other)))
        assertEquals("Open link", LinkOpenPlan.label(emptyList()))
        assertEquals(youtube, LinkOpenPlan.app(listOf(youtube, other)))
        assertNull(LinkOpenPlan.app(emptyList()))
    }

    private class Recorder(private val works: (OpenAttempt) -> Boolean) : LinkLauncher {
        val tried = mutableListOf<Pair<String, OpenAttempt>>()
        override fun launch(url: String, attempt: OpenAttempt): Boolean {
            tried += url to attempt
            return works(attempt)
        }
    }

    private val url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=83s"

    @Test
    fun anAppThatOwnsTheLinkOpensItAndTheBrowserIsNotTried() {
        val launcher = Recorder { it == OpenAttempt.RequireNonBrowser }
        assertTrue(LinkOpener({ listOf(youtube) }, launcher, android17).open(url))
        assertEquals(listOf(url to OpenAttempt.RequireNonBrowser), launcher.tried)
    }

    @Test
    fun withNoAppForTheLinkTheDefaultBrowserOpensIt() {
        val launcher = Recorder { it == OpenAttempt.Default }
        assertTrue(LinkOpener({ emptyList() }, launcher, android17).open(url))
        assertEquals(listOf(OpenAttempt.RequireNonBrowser, OpenAttempt.Default), launcher.tried.map { it.second })
    }

    @Test
    fun onAndroid10TheResolvedAppIsOpenedByPackage() {
        val launcher = Recorder { it is OpenAttempt.InPackage }
        assertTrue(LinkOpener({ listOf(youtube) }, launcher, android10).open(url))
        assertEquals(listOf(OpenAttempt.InPackage("com.google.android.youtube")), launcher.tried.map { it.second })
        val browserOnly = Recorder { it == OpenAttempt.Default }
        assertTrue(LinkOpener({ emptyList() }, browserOnly, android10).open(url))
        assertEquals(listOf(OpenAttempt.Default), browserOnly.tried.map { it.second })
    }

    @Test
    fun whenNothingCanOpenItTheOpenerSaysSoAndTheUrlIsUntouched() {
        val launcher = Recorder { false }
        assertFalse(LinkOpener({ emptyList() }, launcher, android17).open(url))
        assertTrue("the same string every time, timestamp included", launcher.tried.all { it.first == url })
    }

    @Test
    fun aUrlThatIsNotAWebLinkIsNeverStarted() {
        val launcher = Recorder { true }
        for (bad in listOf("javascript:alert(1)", "file:///sdcard/x", "intent://x#Intent;end", "https://exam ple.com", " https://example.com/")) {
            assertFalse(LinkOpener({ listOf(youtube) }, launcher, android17).open(bad))
        }
        assertTrue("not even tried", launcher.tried.isEmpty())
    }
}
