package app.kinrelay.android.share

import android.content.Context
import android.content.Intent
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import app.kinrelay.android.MainActivity
import app.kinrelay.android.R
import app.kinrelay.android.data.DeviceEntity
import app.kinrelay.android.log.KinLog

/**
 * The direct-share shortcut: the paired PC in the share sheet's top row (the sharing shortcuts API, android.md). The system picks it
 * because its category matches the `share-target` in `res/xml/shortcuts.xml`, and a tap on it starts [ShareActivity] with
 * the shared content, like the plain "Kin" entry. It is long-lived, so the share sheet keeps showing it, and it follows the paired
 * PC: renamed when the PC's name changes, removed when the phone is unpaired.
 */
object ShareShortcuts {
    const val CATEGORY = "app.kinrelay.android.category.SEND_TO_PC"
    const val SHORTCUT_ID = "pc"

    fun sync(context: Context, pc: DeviceEntity?, log: KinLog) {
        try {
            if (pc == null) {
                ShortcutManagerCompat.removeAllDynamicShortcuts(context)
                ShortcutManagerCompat.removeLongLivedShortcuts(context, listOf(SHORTCUT_ID))
                log.i("share", "direct-share shortcut removed")
                return
            }
            val shortcut = ShortcutInfoCompat.Builder(context, SHORTCUT_ID)
                .setShortLabel(pc.name.take(MAX_LABEL))
                .setLongLabel(pc.name.take(MAX_LABEL))
                .setIcon(IconCompat.createWithResource(context, R.mipmap.ic_launcher))
                .setCategories(setOf(CATEGORY))
                .setLongLived(true)
                // Not started by a share (the share-target says where that goes), but a shortcut needs an intent with an action.
                .setIntent(Intent(context, MainActivity::class.java).setAction(Intent.ACTION_VIEW))
                .build()
            val pushed = ShortcutManagerCompat.pushDynamicShortcut(context, shortcut)
            log.i("share", "direct-share shortcut for '${pc.name}' pushed=$pushed")
        } catch (e: Exception) {
            log.w("share", "direct-share shortcut failed: ${e.javaClass.simpleName}: ${e.message}")
        }
    }

    private const val MAX_LABEL = 40
}
