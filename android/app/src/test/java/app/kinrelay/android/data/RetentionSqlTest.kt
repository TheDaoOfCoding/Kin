package app.kinrelay.android.data

import androidx.sqlite.SQLiteConnection
import androidx.sqlite.SQLiteStatement
import androidx.sqlite.driver.bundled.BundledSQLiteDriver
import androidx.sqlite.execSQL
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.io.File
import java.nio.file.Files

/**
 * The queries that keep the database small ([QuerySql]) and the ones the cache sweep and the success rate depend on, run on a real SQLite over a
 * database built from the exported schema of version 4. The DAOs use these very strings.
 */
class RetentionSqlTest {
    private val dir = Files.createTempDirectory("kin-retention").toFile()
    private lateinit var db: SQLiteConnection

    @Before
    fun setUp() {
        db = BundledSQLiteDriver().open(File(dir, "kin.db").path)
        val schema = Json.parseToJsonElement(File("schemas/app.kinrelay.android.data.KinDatabase/4.json").readText()).jsonObject.getValue("database").jsonObject
        for (entity in schema.getValue("entities").jsonArray.map { it.jsonObject }) {
            val table = entity.getValue("tableName").jsonPrimitive.content
            db.execSQL(entity.getValue("createSql").jsonPrimitive.content.replace("\${TABLE_NAME}", table))
            for (index in entity["indices"]?.jsonArray.orEmpty()) db.execSQL(index.jsonObject.getValue("createSql").jsonPrimitive.content.replace("\${TABLE_NAME}", table))
        }
    }

    @After
    fun tearDown() {
        db.close()
        dir.deleteRecursively()
    }

    private fun transfer(id: String, at: Long, status: String, direction: String = "up", owed: String? = null, errorCode: String? = null, retriedAs: String? = null, attempts: Int = 1) {
        db.prepare(
            "INSERT INTO transfers (id, direction, peerDeviceId, peerName, kind, status, createdAtMillis, finishedAtMillis, totalBytes, errorCode, message, completeAcked, localId, retriedAs, attempts, owed) " +
                "VALUES (?, ?, 'pc', 'Desk PC', 'file', ?, ?, NULL, 10, ?, NULL, 0, ?, ?, ?, ?)"
        ).use { st ->
            st.bindText(1, id); st.bindText(2, direction); st.bindText(3, status); st.bindLong(4, at)
            if (errorCode == null) st.bindNull(5) else st.bindText(5, errorCode)
            st.bindText(6, "local-$id")
            if (retriedAs == null) st.bindNull(7) else st.bindText(7, retriedAs)
            st.bindLong(8, attempts.toLong())
            if (owed == null) st.bindNull(9) else st.bindText(9, owed)
            st.step()
        }
    }

    private fun file(transferId: String) {
        db.execSQL("INSERT INTO transfer_files VALUES ('$transferId', 'f1', 0, 'a.bin', NULL, 'application/octet-stream', 10, 0, 'uri://$transferId', NULL, 0)")
    }

    private fun text(id: String, at: Long, status: String, body: String? = "hello", direction: String = "up", errorCode: String? = null, retriedAs: String? = null, attempts: Int = 1) {
        db.prepare("INSERT INTO texts VALUES (?, ?, 'pc', 'Desk PC', ?, ?, NULL, ?, 5, 0, 0, 'share', ?, NULL, ?, ?)").use { st ->
            st.bindText(1, id); st.bindText(2, direction); st.bindText(3, status); st.bindLong(4, at)
            if (body == null) st.bindNull(5) else st.bindText(5, body)
            if (errorCode == null) st.bindNull(6) else st.bindText(6, errorCode)
            st.bindLong(7, attempts.toLong())
            if (retriedAs == null) st.bindNull(8) else st.bindText(8, retriedAs)
            st.step()
        }
    }

    private fun run(sql: String, vararg longs: Long) {
        db.prepare(sql).use { st ->
            longs.forEachIndexed { i, v -> st.bindLong(i + 1, v) }
            st.step()
        }
    }

    private fun <T> query(sql: String, read: (SQLiteStatement) -> T): List<T> {
        val rows = mutableListOf<T>()
        db.prepare(sql).use { st -> while (st.step()) rows += read(st) }
        return rows
    }

    private fun idsOf(sql: String) = query(sql) { it.getText(0) }

    // ---------------------------------------------------------------- transfers: IDs stay for ever, file rows go

    @Test
    fun theTransferRowsStayForEverAndTheFileRowsOfOldFinishedOnesGo() {
        // t1..t8, oldest first. keep = 3: t6, t7, t8 are the history; t1..t5 are older.
        transfer("t1", 1, "done"); transfer("t2", 2, "done", owed = "complete"); transfer("t3", 3, "queued")
        transfer("t4", 4, "failed"); transfer("t5", 5, "canceled", owed = "cancel:user"); transfer("t6", 6, "done")
        transfer("t7", 7, "failed"); transfer("t8", 8, "done")
        for (i in 1..8) file("t$i")

        run(QuerySql.DROP_OLD_TRANSFER_FILES, 3)

        assertEquals("every transfer ID is kept: a transfer ID is never used twice", (1..8).map { "t$it" }, idsOf("SELECT id FROM transfers ORDER BY id"))
        assertEquals(
            "files kept: the newest 3, an unfinished one, and the two that still owe the PC a message",
            listOf("t2", "t3", "t5", "t6", "t7", "t8"),
            idsOf("SELECT transferId FROM transfer_files ORDER BY transferId"),
        )
    }

    @Test
    fun theFileRowsGoOnceTheOwedMessageIsAcked() {
        transfer("t1", 1, "done", owed = "complete"); transfer("t2", 2, "done"); file("t1")
        run(QuerySql.DROP_OLD_TRANSFER_FILES, 1)
        assertEquals(listOf("t1"), idsOf("SELECT transferId FROM transfer_files"))
        db.execSQL("UPDATE transfers SET owed = NULL, owedId = NULL WHERE id = 't1'") // what DAO.clearOwed does
        run(QuerySql.DROP_OLD_TRANSFER_FILES, 1)
        assertTrue(idsOf("SELECT transferId FROM transfer_files").isEmpty())
    }

    // ---------------------------------------------------------------- texts: rows stay, old bodies go

    @Test
    fun theBodiesOfFinishedTextsBeyondTheNewest500GoAndTheRowsStay() {
        for (i in 1..520) text("x$i", i.toLong(), if (i == 3) "queued" else "done")
        text("old-failed", 0, "failed")

        run(QuerySql.DROP_OLD_TEXT_BODIES, QueuePolicy.HISTORY_LIMIT.toLong())

        assertEquals("no text row is deleted", 521, query("SELECT COUNT(*) FROM texts") { it.getLong(0) }.single().toInt())
        assertEquals("bodies kept: the newest 500 and the one still queued", 501, query("SELECT COUNT(*) FROM texts WHERE body IS NOT NULL") { it.getLong(0) }.single().toInt())
        assertEquals(listOf("x3"), idsOf("SELECT id FROM texts WHERE body IS NOT NULL AND id IN ('x1','x2','x3','x4','x20')"))
        assertEquals(listOf("x21"), idsOf("SELECT id FROM texts WHERE id = 'x21' AND body IS NOT NULL"))
    }

    // ---------------------------------------------------------------- the cache sweep and the success rate

    @Test
    fun theLiveCacheKeysAreQueuedActiveAndFailedWithFilesAndNotRetried() {
        transfer("q", 1, "queued"); transfer("a", 2, "active"); transfer("f", 3, "failed"); transfer("fnofiles", 4, "failed")
        transfer("r", 5, "failed", retriedAs = "q"); transfer("d", 6, "done"); transfer("c", 7, "canceled")
        for (id in listOf("q", "a", "f", "r", "d", "c")) file(id)

        assertEquals(listOf("local-a", "local-f", "local-q"), idsOf(QuerySql.LIVE_LOCAL_IDS).sorted())
    }

    @Test
    fun aRetriedItemIsOneSendCountedByTheItemThatTookItsPlace() {
        transfer("old", 1, "failed", errorCode = "not_found", retriedAs = "new"); transfer("new", 2, "done", attempts = 2); transfer("plain", 3, "done")
        text("tx-old", 1, "failed", errorCode = "internal", retriedAs = "tx-new"); text("tx-new", 2, "done", attempts = 2); text("tx-plain", 3, "done")
        text("tx-in", 4, "done", direction = "down")

        val transfers = query(QuerySql.OUTGOING_TRANSFER_OUTCOMES) { listOf(it.getText(0), it.getLong(2).toString()) }
        val texts = query(QuerySql.OUTGOING_TEXT_OUTCOMES) { listOf(it.getText(0), it.getLong(2).toString()) }
        assertEquals(listOf(listOf("done", "2"), listOf("done", "1")), transfers.sortedByDescending { it[1] })
        assertEquals(listOf(listOf("done", "2"), listOf("done", "1")), texts.sortedByDescending { it[1] })
    }

    @Test
    fun theQueriesCanUseTheIndexesTheyWereMadeFor() {
        // INDEXED BY makes SQLite refuse a query that can't use the index (a plan on an empty table says nothing: it would scan anyway).
        val queries = listOf(
            "SELECT * FROM transfers INDEXED BY index_transfers_status_createdAtMillis WHERE status IN ('queued', 'active') ORDER BY createdAtMillis",
            "SELECT * FROM transfers INDEXED BY index_transfers_createdAtMillis ORDER BY createdAtMillis DESC LIMIT 500",
            "SELECT * FROM transfers INDEXED BY index_transfers_owed WHERE owed IS NOT NULL AND peerDeviceId = 'pc'",
            "SELECT * FROM texts INDEXED BY index_texts_createdAtMillis ORDER BY createdAtMillis DESC LIMIT 500",
            "SELECT * FROM texts INDEXED BY index_texts_direction_status_createdAtMillis WHERE direction = 'up' AND status = 'queued' ORDER BY createdAtMillis, id",
        )
        for (sql in queries) query(sql) { it.getText(0) }
    }
}
