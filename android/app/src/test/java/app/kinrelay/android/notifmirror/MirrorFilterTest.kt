package app.kinrelay.android.notifmirror

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The skip rules of the design (4.4), every one of them and their order. */
class MirrorFilterTest {
    private val own = "app.kinrelay.android"
    private val policy = MirrorPolicy(enabled = true, allowed = setOf("com.chat", "com.bank.app", "com.example.quiet", own, "com.android.shell"))
    private val filter = MirrorFilter(own, debugChannel = null, flagged = { it == "com.bank.app" })

    private fun n(
        pkg: String = "com.chat", title: String? = "Alice", text: String? = "Hi",
        ongoing: Boolean = false, fgs: Boolean = false, notClearable: Boolean = false, summary: Boolean = false, localOnly: Boolean = false,
        category: String? = null, media: Boolean = false, progressMax: Int = 0, indeterminate: Boolean = false, importance: Int = 3,
        suspended: Boolean = false, visibility: Int = NotifSnapshot.VISIBILITY_PRIVATE, override: Int = NotifSnapshot.VISIBILITY_NO_OVERRIDE,
        user: Boolean = true, redacted: Boolean = false, channel: String? = null,
    ) = NotifSnapshot(
        key = "0|$pkg|1|null|10100", packageName = pkg, channelId = channel, currentUser = user, ongoing = ongoing, foregroundService = fgs, notClearable = notClearable,
        groupSummary = summary, localOnly = localOnly, category = category, media = media, progressMax = progressMax, progressIndeterminate = indeterminate,
        importance = importance, suspended = suspended, visibility = visibility, lockscreenOverride = override, title = title, text = text, redacted = redacted,
    )

    private fun skip(snapshot: NotifSnapshot, p: MirrorPolicy = policy): SkipReason? = (filter.decide(snapshot, p) as? FilterDecision.Skip)?.reason

    @Test
    fun anOrdinaryNotificationOfAnAllowedAppIsSent() {
        assertEquals(FilterDecision.Send(sensitive = false), filter.decide(n(), policy))
    }

    @Test
    fun everyRuleHasItsReason() {
        assertEquals(SkipReason.OWN, skip(n(pkg = own)))
        assertEquals(SkipReason.NOT_ALLOWED, skip(n(pkg = "com.other")))
        assertEquals(SkipReason.OTHER_PROFILE, skip(n(user = false)))
        assertEquals(SkipReason.ONGOING, skip(n(ongoing = true)))
        assertEquals(SkipReason.ONGOING, skip(n(fgs = true)))
        assertEquals(SkipReason.ONGOING, skip(n(notClearable = true)))
        assertEquals(SkipReason.SUMMARY, skip(n(summary = true)))
        assertEquals(SkipReason.LOCAL_ONLY, skip(n(localOnly = true)))
        assertEquals(SkipReason.MEDIA, skip(n(media = true)))
        assertEquals(SkipReason.MEDIA, skip(n(category = "transport")))
        for (category in listOf("progress", "service", "sys", "status", "alarm", "call", "navigation")) assertEquals(category, SkipReason.CATEGORY, skip(n(category = category)))
        assertEquals("a determinate progress bar", SkipReason.CATEGORY, skip(n(progressMax = 100)))
        assertEquals(SkipReason.IMPORTANCE, skip(n(importance = 1)))
        assertEquals(SkipReason.IMPORTANCE, skip(n(suspended = true)))
        assertEquals(SkipReason.SECRET, skip(n(visibility = NotifSnapshot.VISIBILITY_SECRET)))
        assertEquals(SkipReason.EMPTY, skip(n(title = null, text = null)))
    }

    @Test
    fun anIndeterminateProgressBarIsStillAMessage() {
        assertEquals(FilterDecision.Send(false), filter.decide(n(progressMax = 100, indeterminate = true), policy))
    }

    @Test
    fun callsAndAlarmsAreNotMirrored() {
        assertEquals(SkipReason.CATEGORY, skip(n(category = "call", title = "Incoming call", text = "Alice")))
        assertEquals(SkipReason.CATEGORY, skip(n(category = "alarm", title = "Alarm", text = "07:00")))
    }

    @Test
    fun theRulesRunInOrderAndStopAtTheFirstMatch() {
        // Not allowed and ongoing: the first rule that matches is the answer, and it is the allow list.
        assertEquals(SkipReason.NOT_ALLOWED, skip(n(pkg = "com.other", ongoing = true)))
        // Own package beats not-allowed (the own package is in the allow list here, and is still skipped).
        assertEquals(SkipReason.OWN, skip(n(pkg = own, ongoing = true)))
        // Ongoing beats summary beats media beats secret.
        assertEquals(SkipReason.ONGOING, skip(n(ongoing = true, summary = true, media = true)))
        assertEquals(SkipReason.SUMMARY, skip(n(summary = true, localOnly = true)))
        assertEquals(SkipReason.MEDIA, skip(n(media = true, category = "call")))
        assertEquals(SkipReason.CATEGORY, skip(n(category = "call", importance = 1)))
        assertEquals(SkipReason.IMPORTANCE, skip(n(importance = 1, visibility = NotifSnapshot.VISIBILITY_SECRET)))
        assertEquals(SkipReason.SECRET, skip(n(visibility = NotifSnapshot.VISIBILITY_SECRET, title = null, text = null)))
    }

    @Test
    fun aNotificationOfAnAppThatIsNotTickedIsNeverInspected() {
        // The first three rules read only the cheap fields: `pre` is all the service runs before it reads the extras.
        val quick = QuickInfo("com.other", null, true)
        assertEquals(SkipReason.NOT_ALLOWED, filter.pre(quick, policy))
        assertEquals(SkipReason.OTHER_PROFILE, filter.pre(QuickInfo("com.chat", null, false), policy))
        assertEquals(SkipReason.OWN, filter.pre(QuickInfo(own, null, true), policy))
        assertEquals(null, filter.pre(QuickInfo("com.chat", null, true), policy))
    }

    @Test
    fun withTheMasterSwitchOffNothingIsAllowed() {
        assertEquals(SkipReason.NOT_ALLOWED, skip(n(), policy.copy(enabled = false)))
    }

    @Test
    fun theLockScreenChoiceOfTheUserBeatsTheAppsOwn() {
        // The app says private but the user hid it on the lock screen: secret. The app says secret but the user shows it: sent.
        assertEquals(SkipReason.SECRET, skip(n(visibility = NotifSnapshot.VISIBILITY_PRIVATE, override = NotifSnapshot.VISIBILITY_SECRET)))
        assertEquals(FilterDecision.Send(false), filter.decide(n(visibility = NotifSnapshot.VISIBILITY_SECRET, override = NotifSnapshot.VISIBILITY_PUBLIC), policy))
    }

    @Test
    fun aHiddenNotificationOfAnAppTheUserAllowedIsSentAsSensitive() {
        val allowedHidden = policy.copy(hiddenAllowed = setOf("com.chat"))
        assertEquals(FilterDecision.Send(sensitive = true), filter.decide(n(visibility = NotifSnapshot.VISIBILITY_SECRET), allowedHidden))
        assertEquals(SkipReason.SECRET, skip(n(visibility = NotifSnapshot.VISIBILITY_SECRET), policy))
    }

    @Test
    fun aFlaggedAppIsAlwaysSensitive() {
        assertEquals(FilterDecision.Send(sensitive = true), filter.decide(n(pkg = "com.bank.app"), policy))
        assertEquals(FilterDecision.Send(sensitive = false), filter.decide(n(pkg = "com.example.quiet"), policy))
    }

    @Test
    fun aRedactedNotificationWithNoTextIsNotEmpty() {
        assertEquals(FilterDecision.Send(false), filter.decide(n(title = null, text = null, redacted = true), policy))
    }

    @Test
    fun theDebugChannelOfKinItselfIsMirroredInADebugBuildOnly() {
        val debug = MirrorFilter(own, debugChannel = "kin_debug_mirror")
        assertEquals(FilterDecision.Send(false), debug.decide(n(pkg = own, channel = "kin_debug_mirror"), policy))
        assertEquals(FilterDecision.Send(false), debug.decide(n(pkg = own, channel = "kin_debug_mirror_low"), policy))
        assertEquals(SkipReason.OWN, (debug.decide(n(pkg = own, channel = "arrivals"), policy) as FilterDecision.Skip).reason)
        assertEquals(SkipReason.OWN, (filter.decide(n(pkg = own, channel = "kin_debug_mirror"), policy) as FilterDecision.Skip).reason)
    }

    @Test
    fun onlyTheSystemRegroupingRemoveReasonsAreIgnored() {
        assertEquals(setOf(13, 16), MirrorEngine.IGNORED_REMOVE_REASONS)
        assertFalse(10 in MirrorEngine.IGNORED_REMOVE_REASONS) // another listener (a watch) cancelled it: the PC toast goes
        assertFalse(1 in MirrorEngine.IGNORED_REMOVE_REASONS) // a tap (REASON_CLICK) takes the toast away
        assertTrue(2 !in MirrorEngine.IGNORED_REMOVE_REASONS) // REASON_CANCEL
    }
}
