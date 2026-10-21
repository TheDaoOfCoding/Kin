package app.kinrelay.android.debug

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.core.app.RemoteInput
import app.kinrelay.android.AppGraph
import app.kinrelay.android.MainActivity
import app.kinrelay.android.R
import app.kinrelay.android.graph
import app.kinrelay.android.notifmirror.Hashing
import app.kinrelay.android.notifmirror.MirrorController

/**
 * The debug build's test notifications for mirroring (design 4.14), posted from Kin itself on the channel `kin_debug_mirror` (the one channel of
 * Kin's own that the filter doesn't skip, in a debug build only; a release build has neither this file nor that exception). Everything else about
 * them is real: they go through the same listener, filter, encoder and sender as any app's.
 *
 * a: a conversation with 3 messages, a reply, "Mark as read" (a broadcast) and "Open" (an activity: must **not** be offered on the PC);
 * b: a one-time code; c: an ongoing notification; d: a group summary and two children; e: a notification hidden on the lock screen;
 * f: a low-importance one; g: one that looks like Android's "content hidden" placeholder.
 */
object MirrorTestPoster {
    const val CHANNEL = MirrorController.DEBUG_CHANNEL
    const val CHANNEL_LOW = MirrorController.DEBUG_CHANNEL + "_low"
    const val ACTION_REPLY = "app.kinrelay.android.debug.MIRROR_TEST_REPLY"
    const val ACTION_READ = "app.kinrelay.android.debug.MIRROR_TEST_READ"
    const val KEY_REPLY = "reply_text"
    val KINDS = listOf('a', 'b', 'c', 'd', 'e', 'f', 'g')

    private fun ensureChannels(manager: NotificationManager) {
        manager.createNotificationChannel(NotificationChannel(CHANNEL, "Mirror test", NotificationManager.IMPORTANCE_DEFAULT))
        manager.createNotificationChannel(NotificationChannel(CHANNEL_LOW, "Mirror test (low)", NotificationManager.IMPORTANCE_LOW))
    }

    private fun builder(context: Context, channel: String = CHANNEL) =
        NotificationCompat.Builder(context, channel).setSmallIcon(R.drawable.ic_stat_kin).setAutoCancel(true)

    fun post(context: Context, kind: Char, tag: String = "t"): Boolean {
        val manager = context.getSystemService(NotificationManager::class.java)
        ensureChannels(manager)
        when (kind) {
            'a' -> {
                val me = Person.Builder().setName("Me").build()
                val alice = Person.Builder().setName("Alice").build()
                val style = NotificationCompat.MessagingStyle(me)
                    .addMessage("Are you coming tonight?", 1_000L, alice)
                    .addMessage("Yes, at eight", 2_000L, me)
                    .addMessage("Great, bring the charger", 3_000L, alice)
                val remoteInput = RemoteInput.Builder(KEY_REPLY).setLabel("Reply").build()
                val reply = NotificationCompat.Action.Builder(R.drawable.ic_stat_kin, "Reply", receiver(context, ACTION_REPLY, mutable = true))
                    .addRemoteInput(remoteInput).setSemanticAction(NotificationCompat.Action.SEMANTIC_ACTION_REPLY).setShowsUserInterface(false).build()
                val read = NotificationCompat.Action.Builder(R.drawable.ic_stat_kin, "Mark as read", receiver(context, ACTION_READ, mutable = false)).build()
                val open = NotificationCompat.Action.Builder(
                    R.drawable.ic_stat_kin, "Open",
                    PendingIntent.getActivity(context, 1, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE),
                ).build()
                manager.notify(tag, 1, builder(context).setStyle(style).setContentTitle("Alice").setContentText("Great, bring the charger")
                    .addAction(reply).addAction(read).addAction(open).setCategory(Notification.CATEGORY_MESSAGE).build())
            }
            'b' -> manager.notify(tag, 2, builder(context).setContentTitle("Example Service").setContentText("Your verification code is 482913. Don't share it.").setCategory(Notification.CATEGORY_MESSAGE).build())
            'c' -> manager.notify(tag, 3, builder(context).setContentTitle("Ongoing task").setContentText("Running").setOngoing(true).build())
            'd' -> {
                val group = "kin_test_group"
                manager.notify(tag, 4, builder(context).setContentTitle("Group child 1").setContentText("first").setGroup(group).build())
                manager.notify(tag, 5, builder(context).setContentTitle("Group child 2").setContentText("second").setGroup(group).build())
                manager.notify(tag, 6, builder(context).setContentTitle("2 messages").setContentText("summary").setGroup(group).setGroupSummary(true).build())
            }
            'e' -> manager.notify(tag, 7, builder(context).setContentTitle("Secret").setContentText("hidden on the lock screen").setVisibility(NotificationCompat.VISIBILITY_SECRET).build())
            'f' -> manager.notify(tag, 8, builder(context, CHANNEL_LOW).setContentTitle("Low importance").setContentText("quiet").build())
            'g' -> manager.notify(tag, 9, builder(context).setContentTitle("Looks redacted").setContentText("Sensitive notification content hidden").build())
            else -> return false
        }
        return true
    }

    private fun receiver(context: Context, action: String, mutable: Boolean): PendingIntent = PendingIntent.getBroadcast(
        context, if (mutable) 10 else 11, Intent(context, DebugMirrorReceiver::class.java).setAction(action),
        if (mutable) PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT else PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
}

/** What the test notification's buttons did, kept in memory for `MIRROR_STATE`: a reply is only a hash and a length (a test-only echo, never the text). */
object MirrorTestLog {
    @Volatile var replies = 0
    @Volatile var lastReplySha = "none"
    @Volatile var marksRead = 0
}

/** The receiver behind the test notification's Reply and Mark as read. Not exported: only Kin's own `PendingIntent`s reach it. */
class DebugMirrorReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val graph = context.graph
        when (intent.action) {
            MirrorTestPoster.ACTION_REPLY -> {
                val text = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(MirrorTestPoster.KEY_REPLY)?.toString()
                MirrorTestLog.replies++
                MirrorTestLog.lastReplySha = text?.let { Hashing.sha256Hex(it.toByteArray()).substring(0, 8) } ?: "null"
                graph.log.i("debug", "CMD MIRROR_REPLY received sha=${MirrorTestLog.lastReplySha} chars=${text?.length}")
            }
            MirrorTestPoster.ACTION_READ -> {
                MirrorTestLog.marksRead++
                graph.log.i("debug", "CMD MIRROR_MARK_READ received count=${MirrorTestLog.marksRead}")
            }
        }
    }
}

/** The "Mirror test" buttons of the Debug screen. */
@Composable
fun MirrorTestSection(graph: AppGraph, context: Context) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Mirror test (debug build only)", style = MaterialTheme.typography.titleSmall)
        val labels = mapOf(
            'a' to "a: conversation + reply + buttons", 'b' to "b: verification code", 'c' to "c: ongoing", 'd' to "d: group summary + 2",
            'e' to "e: hidden on lock screen", 'f' to "f: low importance", 'g' to "g: looks redacted",
        )
        MirrorTestPoster.KINDS.forEach { kind ->
            OutlinedButton(onClick = { MirrorTestPoster.post(context, kind) }, modifier = Modifier.fillMaxWidth()) { Text(labels.getValue(kind)) }
        }
    }
}
