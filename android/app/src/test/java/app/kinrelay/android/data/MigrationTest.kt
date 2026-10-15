package app.kinrelay.android.data

import androidx.sqlite.SQLiteConnection
import androidx.sqlite.driver.bundled.BundledSQLiteDriver
import androidx.sqlite.execSQL
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files

/**
 * The migrations 2 to 3 (milestone 1.4) and 3 to 4 (the 1.4 review fixes), run on a real SQLite. The old database is built from the
 * **exported schema of the old version** (`app/schemas/.../N.json`, checked in), filled with rows, migrated by the app's own
 * [KinDatabase] migrations, and then compared column by column and index by index with a database built from the exported schema of the new
 * version: the migrated tables must be the tables Room expects. The pairing (`devices`) and every transfer must survive.
 */
class MigrationTest {
    private val dir = Files.createTempDirectory("kin-migration").toFile()
    private val opened = mutableListOf<SQLiteConnection>()

    @After
    fun tearDown() {
        opened.forEach { runCatching { it.close() } }
        dir.deleteRecursively()
    }

    private fun schema(version: Int): JsonObject {
        val file = File("schemas/app.kinrelay.android.data.KinDatabase/$version.json")
        assertTrue("the exported schema $version is checked in (${file.absolutePath})", file.isFile)
        return Json.parseToJsonElement(file.readText()).jsonObject.getValue("database").jsonObject
    }

    private fun open(name: String): SQLiteConnection = BundledSQLiteDriver().open(File(dir, name).path).also { opened += it }

    private fun createAll(connection: SQLiteConnection, schema: JsonObject) {
        for (entity in schema.getValue("entities").jsonArray.map { it.jsonObject }) {
            val table = entity.getValue("tableName").jsonPrimitive.content
            connection.execSQL(entity.getValue("createSql").jsonPrimitive.content.replace("\${TABLE_NAME}", table))
            for (index in entity["indices"]?.jsonArray.orEmpty().map { it.jsonObject }) {
                connection.execSQL(index.getValue("createSql").jsonPrimitive.content.replace("\${TABLE_NAME}", table))
            }
        }
    }

    /** The indexes of a table: name, uniqueness and columns in order. */
    private fun indexes(connection: SQLiteConnection, table: String): List<String> {
        val names = query(connection, "PRAGMA index_list(`$table`)") { it.getText(1) to it.getLong(2) }.filter { !it.first.startsWith("sqlite_autoindex") }
        return names.sortedBy { it.first }.map { (name, unique) ->
            "$name unique=$unique " + query(connection, "PRAGMA index_info(`$name`)") { it.getText(2) }.joinToString(",")
        }
    }

    private data class Column(val name: String, val type: String, val notNull: Boolean, val default: String?, val pk: Int)

    private fun columns(connection: SQLiteConnection, table: String): List<Column> {
        val result = mutableListOf<Column>()
        connection.prepare("PRAGMA table_info(`$table`)").use { st ->
            while (st.step()) {
                result += Column(st.getText(1), st.getText(2), st.getLong(3) == 1L, if (st.isNull(4)) null else st.getText(4), st.getLong(5).toInt())
            }
        }
        return result
    }

    private fun tables(connection: SQLiteConnection): List<String> {
        val names = mutableListOf<String>()
        connection.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name").use { st ->
            while (st.step()) names += st.getText(0)
        }
        return names
    }

    private fun <T> query(connection: SQLiteConnection, sql: String, read: (androidx.sqlite.SQLiteStatement) -> T): List<T> {
        val rows = mutableListOf<T>()
        connection.prepare(sql).use { st -> while (st.step()) rows += read(st) }
        return rows
    }

    @Test
    fun theExportedSchemasAreTheVersionsTheDatabaseDeclares() {
        for (version in 2..4) assertEquals(version, schema(version).getValue("version").jsonPrimitive.content.toInt())
    }

    @Test
    fun migratingV2ToV3KeepsTheDataAndMakesTheTablesRoomExpects() {
        val old = open("old.db")
        createAll(old, schema(2))
        old.execSQL("INSERT INTO devices VALUES ('pc-device-id', 'Desk PC', 'windows', 1000, 2000, '10.0.2.2:47821')")
        old.execSQL("INSERT INTO seen_messages VALUES ('m1', 'ack', 5)")
        old.execSQL("INSERT INTO transfers VALUES ('t-up', 'up', 'pc-device-id', 'Desk PC', 'file', 'failed', 100, 200, 5000, 'connection_lost', 'It failed.', 0)")
        old.execSQL("INSERT INTO transfers VALUES ('t-down', 'down', 'pc-device-id', 'Desk PC', 'file', 'done', 300, 400, 7, NULL, NULL, 1)")
        old.execSQL("INSERT INTO transfer_files VALUES ('t-up', 'f1', 0, 'a.bin', NULL, 'application/octet-stream', 5000, 0, 'file:///cache/queue/t-up/f1', 77, 0)")

        runBlocking { KinDatabase.MIGRATION_2_3.migrate(old) }

        // Nothing was lost: the pairing, the dedup memory, both transfers and the file row.
        assertEquals(listOf("pc-device-id|Desk PC"), query(old, "SELECT deviceId, name FROM devices") { "${it.getText(0)}|${it.getText(1)}" })
        assertEquals(listOf("m1"), query(old, "SELECT id FROM seen_messages") { it.getText(0) })
        // The old rows get what the new code relies on: the cache key is the transfer's own ID, nothing was retried, no attempts counted.
        val rows = query(old, "SELECT id, localId, retriedAs, attempts, status, errorCode FROM transfers ORDER BY id") {
            listOf(it.getText(0), it.getText(1), if (it.isNull(2)) null else it.getText(2), it.getLong(3).toString(), it.getText(4), if (it.isNull(5)) null else it.getText(5))
        }
        assertEquals(
            listOf(
                listOf("t-down", "t-down", null, "0", "done", null),
                listOf("t-up", "t-up", null, "0", "failed", "connection_lost"),
            ),
            rows,
        )
        assertEquals(listOf("file:///cache/queue/t-up/f1"), query(old, "SELECT localUri FROM transfer_files") { it.getText(0) })

        // The migrated database has the tables and columns of a database made from the version 3 schema.
        val fresh = open("fresh.db")
        createAll(fresh, schema(3))
        assertEquals(tables(fresh), tables(old))
        for (table in tables(fresh)) assertEquals("columns of $table", columns(fresh, table), columns(old, table))

        // And the new table works.
        old.execSQL("INSERT INTO texts VALUES ('x1', 'up', 'pc-device-id', 'Desk PC', 'queued', 1, NULL, 'hello', 5, 0, 0, 'share', NULL, NULL, 0)")
        assertEquals(listOf("hello"), query(old, "SELECT body FROM texts") { it.getText(0) })
    }

    @Test
    fun theMigrationStatementsAreWhatTheMigrationRuns() {
        // The statements are listed once: the Migration runs exactly these, in order (a test of the SQL above is a test of the migration).
        assertEquals(5, KinDatabase.MIGRATION_2_3_STATEMENTS.size)
        assertTrue(KinDatabase.MIGRATION_2_3_STATEMENTS.none { it.contains("DROP", ignoreCase = true) })
        assertEquals(2, KinDatabase.MIGRATION_2_3.startVersion)
        assertEquals(3, KinDatabase.MIGRATION_2_3.endVersion)
        assertTrue(KinDatabase.MIGRATION_3_4_STATEMENTS.none { it.contains("DROP", ignoreCase = true) })
        assertEquals(3, KinDatabase.MIGRATION_3_4.startVersion)
        assertEquals(4, KinDatabase.MIGRATION_3_4.endVersion)
    }

    private fun assertSameAsFresh(migrated: SQLiteConnection, version: Int) {
        val fresh = open("fresh$version.db")
        createAll(fresh, schema(version))
        assertEquals(tables(fresh), tables(migrated))
        for (table in tables(fresh)) {
            assertEquals("columns of $table", columns(fresh, table), columns(migrated, table))
            assertEquals("indexes of $table", indexes(fresh, table), indexes(migrated, table))
        }
    }

    @Test
    fun migratingV3ToV4KeepsTheDataAndMakesTheTablesAndIndexesRoomExpects() {
        val old = open("v3.db")
        createAll(old, schema(3))
        old.execSQL("INSERT INTO devices VALUES ('pc-device-id', 'Desk PC', 'windows', 1000, 2000, '10.0.2.2:47821')")
        // columns: id, direction, peerDeviceId, peerName, kind, status, createdAtMillis, finishedAtMillis, totalBytes, errorCode, message, completeAcked, localId, retriedAs, attempts
        old.execSQL("INSERT INTO transfers VALUES ('up-tried', 'up', 'pc-device-id', 'Desk PC', 'file', 'queued', 100, NULL, 50, NULL, NULL, 0, 'up-tried', NULL, 2)")
        old.execSQL("INSERT INTO transfers VALUES ('up-new', 'up', 'pc-device-id', 'Desk PC', 'file', 'queued', 200, NULL, 50, NULL, NULL, 0, 'up-new', NULL, 0)")
        old.execSQL("INSERT INTO transfers VALUES ('down-done', 'down', 'pc-device-id', 'Desk PC', 'file', 'done', 300, 400, 7, NULL, NULL, 1, 'down-done', NULL, 0)")
        old.execSQL("INSERT INTO transfer_files VALUES ('up-tried', 'f1', 0, 'a.bin', NULL, 'application/octet-stream', 50, 0, 'file:///cache/up-tried/f1', 77, 0)")
        old.execSQL("INSERT INTO texts VALUES ('x1', 'up', 'pc-device-id', 'Desk PC', 'failed', 1, 2, 'hello', 5, 0, 0, 'share', 'internal', 'It failed.', 1)")

        runBlocking { KinDatabase.MIGRATION_3_4.migrate(old) }

        // Nothing was lost.
        assertEquals(listOf("pc-device-id|Desk PC"), query(old, "SELECT deviceId, name FROM devices") { "${it.getText(0)}|${it.getText(1)}" })
        assertEquals(3, query(old, "SELECT id FROM transfers") { it.getText(0) }.size)
        assertEquals(listOf("hello"), query(old, "SELECT body FROM texts") { it.getText(0) })
        // The new columns: no retry pending, nothing owed; an upload that had been tried counts as started, one that hadn't does not.
        val rows = query(old, "SELECT id, userRetry, started, owed, owedId FROM transfers ORDER BY id") {
            listOf(it.getText(0), it.getLong(1).toString(), it.getLong(2).toString(), if (it.isNull(3)) null else it.getText(3), if (it.isNull(4)) null else it.getText(4))
        }
        assertEquals(
            listOf(listOf("down-done", "0", "0", null, null), listOf("up-new", "0", "0", null, null), listOf("up-tried", "0", "1", null, null)),
            rows,
        )
        assertEquals(listOf<String?>(null), query(old, "SELECT retriedAs FROM texts") { if (it.isNull(0)) null else it.getText(0) })
        assertSameAsFresh(old, 4)
    }

    @Test
    fun aDatabaseOfVersion2ReachesVersion4ThroughBothMigrations() {
        val old = open("v2.db")
        createAll(old, schema(2))
        old.execSQL("INSERT INTO devices VALUES ('pc-device-id', 'Desk PC', 'windows', 1000, 2000, '10.0.2.2:47821')")
        old.execSQL("INSERT INTO transfers VALUES ('t-up', 'up', 'pc-device-id', 'Desk PC', 'file', 'failed', 100, 200, 5000, 'connection_lost', 'It failed.', 0)")

        runBlocking {
            KinDatabase.MIGRATION_2_3.migrate(old)
            KinDatabase.MIGRATION_3_4.migrate(old)
        }

        assertEquals(listOf("t-up"), query(old, "SELECT id FROM transfers") { it.getText(0) })
        assertSameAsFresh(old, 4)
    }
}
