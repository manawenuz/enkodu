package com.enkodu.companion.data

import android.content.Context
import androidx.room.Database
import androidx.room.Room
import androidx.room.RoomDatabase

@Database(
    entities = [TransferState::class],
    version = 1,
    exportSchema = false
)
abstract class EnkoduDatabase : RoomDatabase() {
    abstract fun transferDao(): TransferDao

    companion object {
        @Volatile
        private var INSTANCE: EnkoduDatabase? = null

        fun getDatabase(context: Context): EnkoduDatabase {
            return INSTANCE ?: synchronized(this) {
                val instance = Room.databaseBuilder(
                    context.applicationContext,
                    EnkoduDatabase::class.java,
                    "enkodu_database"
                ).build()
                INSTANCE = instance
                instance
            }
        }
    }
}
