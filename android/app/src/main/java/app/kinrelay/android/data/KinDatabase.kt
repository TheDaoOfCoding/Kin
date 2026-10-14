package app.kinrelay.android.data

import android.content.Context
import androidx.room3.Database
import androidx.room3.Room
import androidx.room3.RoomDatabase
import androidx.sqlite.driver.AndroidSQLiteDriver

@Database(entities = [DeviceEntity::class, SeenMessageEntity::class], version = 1, exportSchema = false)
abstract class KinDatabase : RoomDatabase() {
    abstract fun devices(): DeviceDao
    abstract fun seenMessages(): SeenMessageDao

    companion object {
        fun create(context: Context): KinDatabase =
            Room.databaseBuilder<KinDatabase>(context.applicationContext, "kin.db")
                .setDriver(AndroidSQLiteDriver())
                .build()
    }
}
