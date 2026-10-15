package app.kinrelay.android.data

import android.content.Context
import androidx.room3.Database
import androidx.room3.Room
import androidx.room3.RoomDatabase
import androidx.room3.migration.Migration
import androidx.sqlite.SQLiteConnection
import androidx.sqlite.driver.AndroidSQLiteDriver
import androidx.sqlite.execSQL

@Database(
    entities = [DeviceEntity::class, SeenMessageEntity::class, TransferEntity::class, TransferFileEntity::class],
    version = 2,
    exportSchema = false,
)
abstract class KinDatabase : RoomDatabase() {
    abstract fun devices(): DeviceDao
    abstract fun seenMessages(): SeenMessageDao
    abstract fun transfers(): TransferDao

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

        fun create(context: Context): KinDatabase =
            Room.databaseBuilder<KinDatabase>(context.applicationContext, "kin.db")
                .setDriver(AndroidSQLiteDriver())
                .addMigrations(MIGRATION_1_2)
                .build()
    }
}
