package app.kinrelay.android.setup

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class OemTableTest {
    @Test
    fun xiaomiRedmiAndPocoGetTheMiuiGuide() {
        for (maker in listOf("Xiaomi", "XIAOMI", "Redmi", "POCO", "  xiaomi ")) assertEquals(maker, "xiaomi", OemTable.select(maker)?.id)
        // Some Redmi and POCO builds say "Xiaomi" as the maker only in the brand.
        assertEquals("xiaomi", OemTable.select("Unknown", "POCO")?.id)
    }

    @Test
    fun samsungGetsNeverSleepingApps() {
        val guide = OemTable.select("samsung")!!
        assertEquals("samsung", guide.id)
        assertTrue(guide.steps.any { it.contains("Never sleeping apps") })
    }

    @Test
    fun oppoRealmeAndOnePlusShareColorOs() {
        for (maker in listOf("OPPO", "realme", "OnePlus")) assertEquals(maker, "coloros", OemTable.select(maker)?.id)
    }

    @Test
    fun vivoAndIqooShareOneGuide() {
        for (maker in listOf("vivo", "iQOO")) assertEquals(maker, "vivo", OemTable.select(maker)?.id)
    }

    @Test
    fun huaweiAndHonorShareOneGuide() {
        for (maker in listOf("HUAWEI", "HONOR")) assertEquals(maker, "huawei", OemTable.select(maker)?.id)
    }

    @Test
    fun otherMakersHaveNoGuideAndGetOnlyTheGeneralSteps() {
        for (maker in listOf("Google", "motorola", "Sony", "", null)) assertNull(maker, OemTable.select(maker))
        assertNull(OemTable.select("Google", "google"))
        assertTrue(OemTable.GENERAL_STEPS.isNotEmpty())
    }

    @Test
    fun theMakerWinsOverTheBrandAndNoMakerIsInTwoGuides() {
        assertEquals("samsung", OemTable.select("samsung", "poco")?.id)
        val all = OemTable.guides.flatMap { it.manufacturers }
        assertEquals("a manufacturer is in one guide only", all.size, all.toSet().size)
    }

    @Test
    fun everyGuideHasStepsAndTargetsThatAreComplete() {
        assertEquals(setOf("xiaomi", "samsung", "coloros", "vivo", "huawei"), OemTable.guides.map { it.id }.toSet())
        for (guide in OemTable.guides) {
            assertTrue("${guide.id}: steps", guide.steps.isNotEmpty())
            assertTrue("${guide.id}: targets", guide.targets.isNotEmpty())
            assertTrue("${guide.id}: lower case makers", guide.manufacturers.all { it == it.lowercase() })
            for (t in guide.targets) {
                assertNotNull("${guide.id}/${t.label}: a package", t.packageName)
                assertTrue("${guide.id}/${t.label}: the class is in the package's activities", t.className!!.contains('.'))
                assertTrue("${guide.id}/${t.label}: a label", t.label.isNotBlank())
            }
        }
    }

    @Test
    fun theXiaomiBatterySaverTargetAsksForKinsOwnPackage() {
        val saver = OemTable.select("xiaomi")!!.targets.first { it.label == "Battery saver" }
        assertEquals("%PACKAGE%", saver.extras["package_name"])
    }
}
