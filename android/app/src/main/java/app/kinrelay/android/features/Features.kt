package app.kinrelay.android.features

import android.content.Context
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import app.kinrelay.android.protocol.Fields

/** The stable ids of the per-feature switches (design section 4). Each device stores, per feature, a default (today's flat key) and an optional override per peer device ID. */
object FeatureIds {
    const val TEXT_AUTO_COPY = "text.autoCopy"
    const val NOTIF_SHARE = "notif.share"
    const val MEDIA_SHARE = "media.share"
    const val MEDIA_SHOW = "media.show"
    const val PRESENCE_SHARE = "presence.share"
    const val PRESENCE_SHOW = "presence.show"
    const val RING_ALLOW = "ring.allow"
    const val SHOTS_SCREENSHOTS = "shots.screenshots"
    const val SHOTS_CAMERA = "shots.camera"

    /** 2B.5: "Silence this phone while my PC is in Focus" (default off) and "Tell my PC when this phone is on Do Not Disturb" (default off). */
    const val DND_FOLLOW = "dnd.follow"
    const val DND_SHARE = "dnd.share"

    val ALL = listOf(
        TEXT_AUTO_COPY, NOTIF_SHARE, MEDIA_SHARE, MEDIA_SHOW,
        PRESENCE_SHARE, PRESENCE_SHOW, RING_ALLOW, SHOTS_SCREENSHOTS, SHOTS_CAMERA,
        DND_FOLLOW, DND_SHARE,
    )
}

/**
 * Per-peer overrides: `{ "<peerDeviceId>": { "<featureId>": true|false } }` (the shape of the `features[]` vectors). Empty in 2B.4 and with no UI; 2D adds the UI and nothing else (no data
 * migration). A device ID is not secret. A value that is not a boolean is ignored.
 */
data class FeatureOverrides(val values: Map<String, Map<String, Boolean>> = emptyMap()) {
    fun forPeer(featureId: String, peerDeviceId: String): Boolean? = values[peerDeviceId]?.get(featureId)

    /** Unlinking a peer deletes its overrides, in the same step that forgets it. */
    fun withoutPeer(peerDeviceId: String): FeatureOverrides = FeatureOverrides(values - peerDeviceId)

    fun with(featureId: String, peerDeviceId: String, enabled: Boolean): FeatureOverrides =
        FeatureOverrides(values + (peerDeviceId to ((values[peerDeviceId] ?: emptyMap()) + (featureId to enabled))))

    fun toJson(): String = buildJsonObject {
        values.forEach { (peer, features) -> put(peer, buildJsonObject { features.forEach { (feature, on) -> put(feature, on) } }) }
    }.toString()

    companion object {
        fun parse(text: String?): FeatureOverrides {
            if (text.isNullOrBlank()) return FeatureOverrides()
            val root = runCatching { Json.parseToJsonElement(text) as? JsonObject }.getOrNull() ?: return FeatureOverrides()
            val out = LinkedHashMap<String, Map<String, Boolean>>()
            for ((peer, features) in root) {
                val featureMap = (features as? JsonObject)?.mapNotNull { (feature, value) -> Fields.boolean(value as? JsonPrimitive)?.let { feature to it } }?.toMap().orEmpty()
                if (featureMap.isNotEmpty()) out[peer] = featureMap
            }
            return FeatureOverrides(out)
        }
    }
}

/** The one function features call: the per-peer override if present, else the default. A feature acting on no peer in particular passes none. */
object FeatureResolver {
    fun enabled(overrides: FeatureOverrides, featureId: String, peerDeviceId: String?, default: Boolean): Boolean =
        peerDeviceId?.let { overrides.forPeer(featureId, it) } ?: default
}

private val Context.featuresStore by preferencesDataStore(name = "kin_features")

/** The override store (`kin_features`, one key `feature_overrides` holding a JSON string). Empty, and no UI in 2B.4. */
class FeatureOverrideStore(context: Context) {
    private val store = context.applicationContext.featuresStore

    val overrides: Flow<FeatureOverrides> = store.data.map { FeatureOverrides.parse(it[OVERRIDES]) }.distinctUntilChanged()

    suspend fun current(): FeatureOverrides = overrides.first()

    suspend fun set(featureId: String, peerDeviceId: String, enabled: Boolean) =
        store.edit { it[OVERRIDES] = FeatureOverrides.parse(it[OVERRIDES]).with(featureId, peerDeviceId, enabled).toJson() }

    /** Called by the unlink path (the same step that forgets the peer). */
    suspend fun forgetPeer(peerDeviceId: String) = store.edit {
        val now = FeatureOverrides.parse(it[OVERRIDES])
        val next = now.withoutPeer(peerDeviceId)
        if (next != now) it[OVERRIDES] = next.toJson()
    }

    private companion object {
        val OVERRIDES = stringPreferencesKey("feature_overrides")
    }
}

/**
 * `Features.enabled(featureId, peer)`: resolves each feature through its override and its default. The defaults are today's flat keys, read by [defaults] (a flow per feature id), so a
 * change of either side reaches the consumers. Every new 2B.4 switch (presence, ring, screenshots) is read **only** through this class (a test checks that no other file names their keys).
 */
class Features(
    private val overrides: FeatureOverrideStore,
    private val defaults: (featureId: String) -> Flow<Boolean>,
) {
    suspend fun enabled(featureId: String, peerDeviceId: String? = null): Boolean =
        FeatureResolver.enabled(overrides.current(), featureId, peerDeviceId, defaults(featureId).first())

    fun flow(featureId: String, peerDeviceId: String? = null): Flow<Boolean> =
        combine(overrides.overrides, defaults(featureId)) { o, d -> FeatureResolver.enabled(o, featureId, peerDeviceId, d) }.distinctUntilChanged()

    suspend fun forgetPeer(peerDeviceId: String) = overrides.forgetPeer(peerDeviceId)
}
