package app.kinrelay.android.data

import android.content.Context
import androidx.room3.Database
import androidx.room3.Room
import androidx.room3.RoomDatabase
import androidx.room3.migration.Migration
import androidx.sqlite.SQLiteConnection
import androidx.sqlite.driver.AndroidSQLiteDriver
import androidx.sqlite.execSQL

/**
 * Version 1: `devices`, `seen_messages`. Version 2 (1.3): `transfers`, `transfer_files`. Version 3 (1.4): `texts` (the text queue and
 * history) and three columns on `transfers`. Version 4 (the 1.4 review fixes): the user-retry and started flags and the owed `complete` or
 * `cancel` on `transfers`, the pointer from a retried text to its successor, and indexes for the queue and history queries. The schema of
 * every version is exported to `app/schemas/` and checked in, so a migration can be tested against the real thing ([MIGRATION_2_3] and
 * [MIGRATION_3_4] have tests that build the old database from `2.json` and `3.json`).
 */
@Database(
    entities = [DeviceEntity::class, SeenMessageEntity::class, TransferEntity::class, TransferFileEntity::class, TextItemEntity::class],
    version = 4,
    exportSchema = true,
)
abstract class KinDatabase : RoomDatabase() {
    abstract fun devices(): DeviceDao
    abstract fun seenMessages(): SeenMessageDao
    abstract fun transfers(): TransferDao
    abstract fun texts(): TextDao

    companion object {
        /**
         * 1.3 adds the transfer tables. A real migration, not a destructive one: the `devices` table holds the pairing, and
         * wiping it would unpair the phone.
         */
        val MIGRATION_1_2 = object : Migration(1, 2) {
            override suspend fun migrate(connection: SQLiteConnection) {
                connection.execSQL(
                    "CREATE TABLE IF NOT EXISTS `transfers` (`id` TEXT NOT NULL, `direction` TEXT NOT NULL, " +
                        "`peerDeviceId` TEXT NOT NULL, `peerName` TEXT NOT NULL, `kind` TEXT NOT NULL, `status` TEXT NOT NULL, " +
                        "`createdAtMillis` INTEGER NOT NULL, `finishedAtMillis` INTEGER, `totalBytes` INTEGER NOT NULL, " +
                        "`errorCode` TEXT, `message` TEXT, `completeAcked` INTEGER NOT NULL, PRIMARY KEY(`id`))"
                )
                connection.execSQL(
                    "CREATE TABLE IF NOT EXISTS `transfer_files` (`transferId` TEXT NOT NULL, `fileId` TEXT NOT NULL, " +
                        "`ordinal` INTEGER NOT NULL, `name` TEXT NOT NULL, `finalName` TEXT, `mime` TEXT NOT NULL, " +
                        "`size` INTEGER NOT NULL, `bytesDone` INTEGER NOT NULL, `localUri` TEXT, `sourceModifiedMillis` INTEGER, " +
                        "`finished` INTEGER NOT NULL, PRIMARY KEY(`transferId`, `fileId`))"
                )
            }
        }

        /**
         * 1.4: the cache key of a transfer (`localId`, filled with the transfer's own ID for the rows that exist), the pointer from a
         * retried transfer to its successor, the attempt counter, and the `texts` table. Nothing is dropped.
         */
        val MIGRATION_2_3 = object : Migration(2, 3) {
            override suspend fun migrate(connection: SQLiteConnection) {
                MIGRATION_2_3_STATEMENTS.forEach { connection.execSQL(it) }
            }
        }

        /** The statements of [MIGRATION_2_3], in order (a test runs them through a real SQLite). */
        val MIGRATION_2_3_STATEMENTS = listOf(
            "ALTER TABLE `transfers` ADD COLUMN `localId` TEXT NOT NULL DEFAULT ''",
            "UPDATE `transfers` SET `localId` = `id`",
            "ALTER TABLE `transfers` ADD COLUMN `retriedAs` TEXT",
            "ALTER TABLE `transfers` ADD COLUMN `attempts` INTEGER NOT NULL DEFAULT 0",
            "CREATE TABLE IF NOT EXISTS `texts` (`id` TEXT NOT NULL, `direction` TEXT NOT NULL, `peerDeviceId` TEXT NOT NULL, " +
                "`peerName` TEXT NOT NULL, `status` TEXT NOT NULL, `createdAtMillis` INTEGER NOT NULL, `finishedAtMillis` INTEGER, " +
                "`body` TEXT, `byteSize` INTEGER NOT NULL, `sensitive` INTEGER NOT NULL, `isUrl` INTEGER NOT NULL, " +
                "`source` TEXT NOT NULL, `errorCode` TEXT, `message` TEXT, `attempts` INTEGER NOT NULL, PRIMARY KEY(`id`))",
        )

        /**
         * The review fixes: `userRetry` (a Retry made offline stays the user's until its `POST`), `started` (an upload the PC has answered
         * is not expired by the 7 days of the queue), `owed` and `owedId` (a `complete` or `cancel` nobody acked yet), the pointer from a
         * retried text to its successor, and the indexes. An upload that was already tried is marked started. Nothing is dropped.
         */
        val MIGRATION_3_4 = object : Migration(3, 4) {
            override suspend fun migrate(connection: SQLiteConnection) {
                MIGRATION_3_4_STATEMENTS.forEach { connection.execSQL(it) }
            }
        }

        /** The statements of [MIGRATION_3_4], in order (a test runs them through a real SQLite). */
        val MIGRATION_3_4_STATEMENTS = listOf(
            "ALTER TABLE `transfers` ADD COLUMN `userRetry` INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE `transfers` ADD COLUMN `started` INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE `transfers` ADD COLUMN `owed` TEXT",
            "ALTER TABLE `transfers` ADD COLUMN `owedId` TEXT",
            "UPDATE `transfers` SET `started` = 1 WHERE `direction` = 'up' AND `attempts` > 0",
            "ALTER TABLE `texts` ADD COLUMN `retriedAs` TEXT",
            "CREATE INDEX IF NOT EXISTS `index_transfers_status_createdAtMillis` ON `transfers` (`status`, `createdAtMillis`)",
            "CREATE INDEX IF NOT EXISTS `index_transfers_createdAtMillis` ON `transfers` (`createdAtMillis`)",
            "CREATE INDEX IF NOT EXISTS `index_transfers_owed` ON `transfers` (`owed`)",
            "CREATE INDEX IF NOT EXISTS `index_texts_createdAtMillis` ON `texts` (`createdAtMillis`)",
            "CREATE INDEX IF NOT EXISTS `index_texts_direction_status_createdAtMillis` ON `texts` (`direction`, `status`, `createdAtMillis`)",
        )

        fun create(context: Context): KinDatabase =
            Room.databaseBuilder<KinDatabase>(context.applicationContext, "kin.db")
                .setDriver(AndroidSQLiteDriver())
                .addMigrations(MIGRATION_1_2, MIGRATION_2_3, MIGRATION_3_4)
                .build()
    }
}
