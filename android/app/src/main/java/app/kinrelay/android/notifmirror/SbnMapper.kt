package app.kinrelay.android.notifmirror

import android.app.Notification
import android.app.PendingIntent
import android.os.Build
import android.os.Process
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import androidx.core.app.NotificationCompat

/**
 * `StatusBarNotification` and its `Ranking` to a [NotifSnapshot] (design 4.1, 4.5). Thin on purpose: every decision is made on the snapshot.
 * [quick] reads only the cheap fields (rules 1 to 3); [snapshot] reads the extras and runs only for an allowed app. **Never read:** the large
 * icon (a contact's photo), pictures, `EXTRA_PEOPLE`, `Person.key`, bubble metadata, custom views and the `dataUri` of a message.
 */
object SbnMapper {
    fun quick(sbn: StatusBarNotification): QuickInfo = QuickInfo(
        packageName = sbn.packageName,
        channelId = sbn.notification?.channelId,
        currentUser = sbn.user == Process.myUserHandle(),
    )

    fun snapshot(sbn: StatusBarNotification, ranking: NotificationListenerService.Ranking?): NotifSnapshot {
        val n = sbn.notification
        val extras = n.extras
        val flags = n.flags
        val style = runCatching { NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(n) }.getOrNull()
        val user = style?.user
        val messages = style?.messages.orEmpty().map { m ->
            val person = m.person
            val own = person == null || person == user || (person.key != null && person.key == user?.key)
            MessageSnap(
                sender = if (own) null else person?.name?.toString()?.takeIf { it.isNotBlank() } ?: "Someone",
                text = m.text?.toString(),
                timestamp = m.timestamp,
                mime = m.dataMimeType,
            )
        }
        val lines = extras.getCharSequenceArray(Notification.EXTRA_TEXT_LINES)?.map { it.toString() }.orEmpty()
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()
        val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString()
        val bigText = extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString()
        val redacted = RedactionMatcher.isRedacted(Build.VERSION.SDK_INT, listOf(text, bigText) + messages.map { it.text } + lines)
        return NotifSnapshot(
            key = sbn.key,
            packageName = sbn.packageName,
            channelId = n.channelId,
            currentUser = sbn.user == Process.myUserHandle(),
            ongoing = flags and Notification.FLAG_ONGOING_EVENT != 0,
            foregroundService = flags and Notification.FLAG_FOREGROUND_SERVICE != 0,
            notClearable = !sbn.isClearable,
            groupSummary = flags and Notification.FLAG_GROUP_SUMMARY != 0,
            localOnly = flags and Notification.FLAG_LOCAL_ONLY != 0,
            onlyAlertOnce = flags and Notification.FLAG_ONLY_ALERT_ONCE != 0,
            category = n.category,
            media = extras.containsKey(Notification.EXTRA_MEDIA_SESSION) || extras.getString(Notification.EXTRA_TEMPLATE) == MEDIA_STYLE,
            progressMax = extras.getInt(Notification.EXTRA_PROGRESS_MAX, 0),
            progressIndeterminate = extras.getBoolean(Notification.EXTRA_PROGRESS_INDETERMINATE, false),
            importance = ranking?.importance ?: 3,
            suspended = ranking?.isSuspended == true,
            visibility = n.visibility,
            // `Ranking.getLockscreenVisibilityOverride` exists from API 31 (a NoSuchMethodError on Android 10 killed the process: found on the API 29 emulator, caught by lint's NewApi).
            lockscreenOverride = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) ranking?.lockscreenVisibilityOverride ?: NotifSnapshot.VISIBILITY_NO_OVERRIDE else NotifSnapshot.VISIBILITY_NO_OVERRIDE,
            matchesInterruptionFilter = ranking?.matchesInterruptionFilter() ?: true,
            title = title,
            text = text,
            bigText = bigText,
            subText = extras.getCharSequence(Notification.EXTRA_SUB_TEXT)?.toString(),
            conversationTitle = style?.conversationTitle?.toString(),
            isGroupConversation = style?.isGroupConversation == true,
            messages = messages,
            lines = lines,
            personHasContactUri = style?.messages.orEmpty().any { it.person?.uri != null },
            actions = actionsOf(n),
            redacted = redacted,
        )
    }

    private fun actionsOf(n: Notification): List<ActionSnap> = n.actions.orEmpty().mapIndexed { i, a ->
        ActionSnap(
            index = i,
            title = a.title?.toString(),
            semantic = a.semanticAction,
            remoteInputs = a.remoteInputs.orEmpty().map { RemoteInputSnap(it.resultKey, it.label?.toString(), it.allowFreeFormInput, !it.choices.isNullOrEmpty()) },
            hasIntent = a.actionIntent != null,
            isActivity = isActivity(a.actionIntent),
            contextual = a.isContextual,
        )
    }

    /** `PendingIntent.isActivity` exists from API 31; before that nothing can be said. */
    private fun isActivity(intent: PendingIntent?): Boolean? =
        if (intent == null || Build.VERSION.SDK_INT < Build.VERSION_CODES.S) null else intent.isActivity

    private const val MEDIA_STYLE = "android.app.Notification\$MediaStyle"
}
