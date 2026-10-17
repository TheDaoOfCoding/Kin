package app.kinrelay.android.notifmirror

/** Why a notification is not mirrored. [code] is the name used in the counters, the logs and the shared `skip[]` vectors. */
enum class SkipReason(val code: String) {
    OWN("own"),
    NOT_ALLOWED("not_allowed"),
    OTHER_PROFILE("other_profile"),
    ONGOING("ongoing"),
    SUMMARY("summary"),
    LOCAL_ONLY("local_only"),
    MEDIA("media"),
    CATEGORY("category"),
    IMPORTANCE("importance"),
    SECRET("secret"),
    EMPTY("empty"),
}

/** What the filter decided. [Send.sensitive]: the PC shows no preview (a flagged app, or a lock-screen-hidden one the user allowed). */
sealed interface FilterDecision {
    data class Send(val sensitive: Boolean) : FilterDecision
    data class Skip(val reason: SkipReason) : FilterDecision
}

/**
 * The user's choices that the rules read: the master switch, the apps ticked, the apps ticked for notifications hidden on the lock screen,
 * and "Dismiss on the phone too". Stored in DataStore, nothing else about notifications is.
 */
data class MirrorPolicy(
    val enabled: Boolean = false,
    val allowed: Set<String> = emptySet(),
    val hiddenAllowed: Set<String> = emptySet(),
    val dismissOnPhone: Boolean = true,
) {
    fun allows(packageName: String): Boolean = enabled && packageName in allowed
}

/** The cheap fields: the first three rules read nothing else, so a notification of an app that is not ticked is never inspected. */
data class QuickInfo(val packageName: String, val channelId: String?, val currentUser: Boolean)

/**
 * The skip rules of the design (4.4), in order, stopping at the first match. Pure: [ownPackage] is Kin's package, [debugChannel] the
 * channels of Kin's own (that name and any that starts with it) that a debug build still mirrors (null in release), [flagged] says whether an app is a likely-sensitive one.
 */
class MirrorFilter(
    private val ownPackage: String,
    private val debugChannel: String? = null,
    private val flagged: (String) -> Boolean = { false },
) {
    /** Rules 1 to 3. Null means "go on": the full snapshot may be read now. */
    fun pre(quick: QuickInfo, policy: MirrorPolicy): SkipReason? = when {
        quick.packageName == ownPackage && !(debugChannel != null && quick.channelId?.startsWith(debugChannel) == true) -> SkipReason.OWN
        !policy.allows(quick.packageName) -> SkipReason.NOT_ALLOWED
        !quick.currentUser -> SkipReason.OTHER_PROFILE
        else -> null
    }

    /** Rules 4 to 11 and the `sensitive` outcome, for a notification that passed [pre]. */
    fun post(n: NotifSnapshot, policy: MirrorPolicy): FilterDecision {
        if (n.ongoing || n.foregroundService || n.notClearable) return FilterDecision.Skip(SkipReason.ONGOING)
        if (n.groupSummary) return FilterDecision.Skip(SkipReason.SUMMARY)
        if (n.localOnly) return FilterDecision.Skip(SkipReason.LOCAL_ONLY)
        if (n.media || n.category == CATEGORY_TRANSPORT) return FilterDecision.Skip(SkipReason.MEDIA)
        if (n.category in SKIPPED_CATEGORIES || (n.progressMax > 0 && !n.progressIndeterminate)) return FilterDecision.Skip(SkipReason.CATEGORY)
        if (n.importance <= IMPORTANCE_MIN || n.suspended) return FilterDecision.Skip(SkipReason.IMPORTANCE)
        val secret = n.effectiveVisibility == NotifSnapshot.VISIBILITY_SECRET
        if (secret && n.packageName !in policy.hiddenAllowed) return FilterDecision.Skip(SkipReason.SECRET)
        if (!n.hasContent && !n.redacted) return FilterDecision.Skip(SkipReason.EMPTY)
        return FilterDecision.Send(sensitive = flagged(n.packageName) || secret)
    }

    /** All rules on a full snapshot (the shared vectors and the tests; the service calls [pre] first and reads the snapshot only after it). */
    fun decide(n: NotifSnapshot, policy: MirrorPolicy): FilterDecision {
        pre(QuickInfo(n.packageName, n.channelId, n.currentUser), policy)?.let { return FilterDecision.Skip(it) }
        return post(n, policy)
    }

    companion object {
        const val IMPORTANCE_MIN = 1
        const val CATEGORY_TRANSPORT = "transport"

        /** `Notification.CATEGORY_*` values: progress, service, system, status, alarm, call, navigation (design 4.4, rule 8). Calls and alarms are never mirrored in 0.2.0. */
        val SKIPPED_CATEGORIES = setOf("progress", "service", "sys", "status", "alarm", "call", "navigation")
    }
}
