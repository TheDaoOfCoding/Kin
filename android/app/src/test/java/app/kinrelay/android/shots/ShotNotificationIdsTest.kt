package app.kinrelay.android.shots

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

class ShotNotificationIdsTest {
    @Test
    fun twoScreenshotsAThousandIdsApartGetDifferentSlots() {
        // The old scheme was 5000 + id % 1000: these pairs shared a slot.
        for (id in listOf(1L, 77L, 999L, 1234L, 5_000_000L)) {
            assertNotEquals(ShotNotificationIds.of(id), ShotNotificationIds.of(id + 1000))
            assertNotEquals(ShotNotificationIds.of(id), ShotNotificationIds.of(id + 2000))
        }
    }

    @Test
    fun noTwoOfAHundredThousandConsecutiveIdsShareASlot() {
        val seen = HashSet<Int>()
        for (id in 1L..100_000L) assertEquals("id $id collided", true, seen.add(ShotNotificationIds.of(id)))
    }

    @Test
    fun theSlotIsStableForOneScreenshotAndIdsPastThirtyTwoBitsStillDiffer() {
        assertEquals(ShotNotificationIds.of(42L), ShotNotificationIds.of(42L))
        val big = 1L shl 33
        assertNotEquals(ShotNotificationIds.of(big), ShotNotificationIds.of(big + 1))
        assertNotEquals(ShotNotificationIds.of(big), ShotNotificationIds.of(big + (1L shl 32)))
    }
}
