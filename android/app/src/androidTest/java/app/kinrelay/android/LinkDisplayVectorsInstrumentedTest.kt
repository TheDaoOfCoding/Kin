package app.kinrelay.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import app.kinrelay.android.links.LinkDomain
import app.kinrelay.android.protocol.LinkWire
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * `display[]` of `protocol/links.json` (changelog 55) on the device's own `android.icu` (the app's `AndroidUts46`). The JVM unit tests run the same
 * vectors on ICU4J because `android.icu` is a stub there; the ICU version differs by Android version, so this runs on the oldest (API 29) and the
 * newest (API 37) emulator. The vectors are assets of the test APK (`../../protocol`).
 *
 *   gradlew connectedDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=app.kinrelay.android.LinkDisplayVectorsInstrumentedTest
 */
@RunWith(AndroidJUnit4::class)
class LinkDisplayVectorsInstrumentedTest {
    @Test
    fun everyDisplayVectorOnTheDevicesIcu() {
        val text = InstrumentationRegistry.getInstrumentation().context.assets.open("links.json").bufferedReader(Charsets.UTF_8).use { it.readText() }
        val cases = JSONObject(text).getJSONArray("display")
        assertTrue("there are ${cases.length()} display cases", cases.length() >= 46)
        val failures = mutableListOf<String>()
        for (i in 0 until cases.length()) {
            val case = cases.getJSONObject(i)
            val name = case.getString("name")
            val url = case.getString("url")
            if (!LinkWire.isLinkUrl(url)) { failures += "$name: the rule rejects the url"; continue }
            val address = LinkDomain.of(url)
            if (address == null) { failures += "$name: no address"; continue }
            if (address.display != case.getString("display")) failures += "$name: display '${address.display}', expected '${case.getString("display")}'"
            if (address.warn != case.getBoolean("warn")) failures += "$name: warn ${address.warn}, expected ${case.getBoolean("warn")}"
        }
        assertEquals("API ${android.os.Build.VERSION.SDK_INT}: " + failures.joinToString("; "), 0, failures.size)
    }
}
