package com.enkodu.companion

import android.app.Application
import androidx.work.Configuration
import com.enkodu.companion.auth.AuthRepository
import com.enkodu.companion.auth.AuthStore
import com.enkodu.companion.data.EnkoduDatabase

class EnkoduApp : Application(), Configuration.Provider {

    val database: EnkoduDatabase by lazy {
        EnkoduDatabase.getDatabase(this)
    }

    val authStore: AuthStore by lazy {
        AuthStore(this)
    }

    val authRepository: AuthRepository by lazy {
        AuthRepository(authStore)
    }

    val settingsStore: SettingsStore by lazy {
        SettingsStore(this)
    }

    override val workManagerConfiguration: Configuration
        get() = Configuration.Builder()
            .setMinimumLoggingLevel(android.util.Log.INFO)
            .build()
}
