package app.kinrelay.android.link

import android.content.Intent
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** 0.2.1: the service comes back after an update of the app, not only after a reboot. */
class BootReceiverTest {
    @Test
    fun aRebootAndAnUpdateRestartTheServiceAndNothingElseDoes() {
        assertTrue(BootReceiver.restartsService(Intent.ACTION_BOOT_COMPLETED))
        assertTrue(BootReceiver.restartsService(Intent.ACTION_MY_PACKAGE_REPLACED))
        assertFalse(BootReceiver.restartsService(null))
        assertFalse(BootReceiver.restartsService("android.intent.action.PACKAGE_REPLACED"))
        assertFalse(BootReceiver.restartsService(Intent.ACTION_USER_PRESENT))
    }

    /** The receiver only gets the broadcasts its filter names: the two actions have to be in the manifest, on a receiver that is not exported. */
    @Test
    fun theManifestDeliversBothBroadcastsToTheReceiverAndExportsNothing() {
        val manifest = File("src/main/AndroidManifest.xml").readText()
        val start = manifest.indexOf("android:name=\".link.BootReceiver\"")
        assertTrue("the BootReceiver entry", start >= 0)
        val receiver = manifest.substring(start, manifest.indexOf("</receiver>", start))
        assertTrue(receiver.contains("android:exported=\"false\""))
        assertEquals(1, Regex("android.intent.action.BOOT_COMPLETED").findAll(receiver).count())
        assertEquals(1, Regex("android.intent.action.MY_PACKAGE_REPLACED").findAll(receiver).count())
    }
}
