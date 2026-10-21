package app.kinrelay.android.pairing

import app.kinrelay.android.identity.DeviceIds
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The rule that a pairing link from outside the app never pairs without a tap on Confirm. */
class PairingIntakeTest {
    private val pcId = "laxXyRYeJ8c1m5mzyG3nUqkN2wQZ4VTh6bP0d9sIfEo"
    private val otherId = "nrOqgVDbtcLFKV2mp2ScXpkjV82JOMfeh4JSL7rP61Q"
    private val token = "AAECAwQFBgcICQoLDA0ODw"

    private fun link(id: String = pcId, name: String? = "Desk%20PC") =
        "kinrelay://pair?v=1&a=10.0.2.2:47821&id=$id&tk=$token" + (name?.let { "&n=$it" } ?: "")

    private fun decide(
        text: String,
        source: PairingSource,
        current: Pair<String, String>? = null,
        autoConfirm: Boolean = false,
    ) = PairingIntake.decide(PairingUrl.parse(text), source, current, autoConfirm)

    @Test
    fun aLinkFromOutsideTheAppAsksFirstAndDoesNotPair() {
        val decision = decide(link(), PairingSource.EXTERNAL_LINK)
        assertTrue("got $decision", decision is PairingIntake.AskUser)
    }

    @Test
    fun theConfirmationShowsTheNameAndTheShortIdWorkedOutFromTheId() {
        val c = (decide(link(otherId), PairingSource.EXTERNAL_LINK) as PairingIntake.AskUser).confirmation
        assertEquals("Desk PC", c.pcName)
        assertEquals("T2Z2-VAKQ", c.shortId) // the fixture's short ID, T2Z2VAKQ
        assertEquals(DeviceIds.display(DeviceIds.shortIdOfDeviceId(otherId)!!), c.shortId)
        assertEquals(otherId, c.target.id)
    }

    @Test
    fun aLinkWithoutANameStillAsks() {
        val c = (decide(link(name = null), PairingSource.EXTERNAL_LINK) as PairingIntake.AskUser).confirmation
        assertEquals(PairingIntake.UNKNOWN_PC_NAME, c.pcName)
    }

    @Test
    fun itSaysWhichPcWillBeReplaced() {
        val c = (decide(link(otherId), PairingSource.EXTERNAL_LINK, current = pcId to "Office PC") as PairingIntake.AskUser).confirmation
        assertEquals("Office PC", c.replacing)
        assertEquals(false, c.alreadyPaired)
    }

    @Test
    fun nothingIsReplacedWhenNothingIsPaired() {
        val c = (decide(link(), PairingSource.EXTERNAL_LINK, current = null) as PairingIntake.AskUser).confirmation
        assertNull(c.replacing)
    }

    @Test
    fun theSamePcAgainIsARefreshNotAReplacement() {
        val c = (decide(link(pcId), PairingSource.EXTERNAL_LINK, current = pcId to "Desk PC") as PairingIntake.AskUser).confirmation
        assertNull(c.replacing)
        assertEquals(true, c.alreadyPaired)
    }

    @Test
    fun theHttpsFormFromOutsideAsksToo() {
        val https = "https://example.com/pair#v=1&a=10.0.2.2:47821&id=$pcId&tk=$token&n=Evil%20PC"
        val c = (decide(https, PairingSource.EXTERNAL_LINK, current = otherId to "My PC") as PairingIntake.AskUser).confirmation
        assertEquals("Evil PC", c.pcName)
        assertEquals("My PC", c.replacing)
    }

    @Test
    fun aLinkFromOutsideIsConfirmedEvenWhenItIsNotTheFirst() {
        // Two links in a row: each one asks. Nothing is remembered as "already allowed".
        repeat(3) { assertTrue(decide(link(), PairingSource.EXTERNAL_LINK) is PairingIntake.AskUser) }
    }

    @Test
    fun whatTheUserScannedOrPastedInTheAppStartsRightAway() {
        val decision = decide(link(), PairingSource.IN_APP)
        assertTrue(decision is PairingIntake.PairNow)
        assertEquals(pcId, (decision as PairingIntake.PairNow).target.id)
    }

    @Test
    fun theDebugAutoConfirmSkipsTheScreenOnlyWhenTheBuildPassesTrue() {
        assertTrue(decide(link(), PairingSource.EXTERNAL_LINK, autoConfirm = true) is PairingIntake.PairNow)
        assertTrue(decide(link(), PairingSource.EXTERNAL_LINK, autoConfirm = false) is PairingIntake.AskUser)
    }

    @Test
    fun badLinksAreRejectedWithAMessageAndNeverAsk() {
        for (source in PairingSource.entries) {
            for (text in listOf("https://example.com/other", "kinrelay://pair?v=1", "kinrelay://pair?v=2&a=10.0.2.2:1&id=$pcId&tk=$token", "garbage")) {
                val decision = decide(text, source, autoConfirm = true)
                assertTrue("$source $text -> $decision", decision is PairingIntake.Reject)
                assertTrue((decision as PairingIntake.Reject).message.isNotBlank())
            }
        }
    }

    // --- the tap confirms the link that was on screen, not whichever one is pending now ---

    private fun confirmation(id: String, name: String) =
        (decide(link(id, name), PairingSource.EXTERNAL_LINK) as PairingIntake.AskUser).confirmation

    @Test
    fun confirmingTheLinkThatWasShownGivesItsTarget() {
        val shown = confirmation(pcId, "Desk%20PC")
        val target = PairingIntake.confirmable(PairingUiState.Confirm(shown), shown)
        assertEquals(pcId, target?.id)
    }

    @Test
    fun aLinkThatReplacedTheShownOneIsNotConfirmedByAnOldTap() {
        val shown = confirmation(pcId, "Desk%20PC")
        val replacedBy = confirmation(otherId, "Attacker")
        // The user is looking at `shown`, a second link arrives, and the tap lands after the state changed.
        assertNull(PairingIntake.confirmable(PairingUiState.Confirm(replacedBy), shown))
    }

    @Test
    fun nothingIsConfirmedWhenNoConfirmationIsWaiting() {
        val shown = confirmation(pcId, "Desk%20PC")
        assertNull(PairingIntake.confirmable(PairingUiState.Idle, shown))
        assertNull(PairingIntake.confirmable(PairingUiState.Working("x"), shown))
    }
}
