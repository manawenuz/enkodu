package com.enkodu.companion

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.navigation.NavHostController
import com.enkodu.companion.av1.Av1CapabilityChecker
import com.enkodu.companion.transfer.UploadWorker
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
fun HomeScreen(navController: NavHostController) {
    val context = LocalContext.current
    val activity = context as ComponentActivity
    val app = context.applicationContext as EnkoduApp
    val settings = app.settingsStore
    val authStore = app.authStore
    val authConfig by authStore.authConfig.collectAsState()
    val picker = remember { VideoPicker(activity) }
    var isUpgradeEnabled by remember { mutableStateOf(false) }
    var isChecking by remember { mutableStateOf(true) }

    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) {
            val result = Av1CapabilityChecker.check()
            isUpgradeEnabled = result.supported
            isChecking = false
        }
    }

    if (!authConfig.isConfigured()) {
        OnboardingScreen(
            onComplete = {}
        )
    } else if (isChecking) {
        Column(
            modifier = Modifier.fillMaxSize(),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Text("Checking device capability...")
        }
    } else if (!isUpgradeEnabled) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(24.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Text(
                "AV1 Not Supported",
                style = MaterialTheme.typography.headlineSmall
            )
            Text(
                "This device cannot play AV1 efficiently. The upgrade flow is disabled.",
                modifier = Modifier.padding(top = 16.dp),
                style = MaterialTheme.typography.bodyMedium
            )
            Text(
                "Queue status and monitoring are still available.",
                modifier = Modifier.padding(top = 8.dp),
                style = MaterialTheme.typography.bodySmall
            )
            Button(
                onClick = { navController.navigate("queue") },
                modifier = Modifier.padding(top = 24.dp)
            ) {
                Text("View Queue")
            }
            Button(
                onClick = { navController.navigate("settings") },
                modifier = Modifier.padding(top = 8.dp)
            ) {
                Text("Settings")
            }
        }
    } else {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(24.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Text(
                "Ready to upgrade videos",
                style = MaterialTheme.typography.headlineSmall
            )
            Spacer(modifier = Modifier.height(16.dp))
            Text(
                "Pick a video to upload for AV1 conversion",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Button(
                onClick = {
                    if (!authConfig.isConfigured()) {
                        navController.navigate("settings")
                        return@Button
                    }
                    picker.pickVideo { file ->
                        if (file != null) {
                            UploadWorker.schedule(
                                context = context,
                                filePath = file.absolutePath,
                                serverUrl = authConfig.serverUrl,
                                requiresWifi = settings.wifiOnlyUploads
                            )
                        }
                    }
                },
                modifier = Modifier.padding(top = 24.dp)
            ) {
                Text("Select Video")
            }
            Button(
                onClick = { navController.navigate("queue") },
                modifier = Modifier.padding(top = 8.dp)
            ) {
                Text("View Queue")
            }
            Button(
                onClick = { navController.navigate("settings") },
                modifier = Modifier.padding(top = 8.dp)
            ) {
                Text("Settings")
            }
        }
    }
}
