package com.enkodu.companion

import android.widget.Toast
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Divider
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.navigation.NavHostController
import com.enkodu.companion.auth.AuthConfig
import com.enkodu.companion.auth.AuthConfigValidator
import com.enkodu.companion.auth.AuthConnectionState
import com.enkodu.companion.auth.AuthMode
import com.enkodu.companion.auth.AuthProbeResult
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(navController: NavHostController) {
    val context = LocalContext.current
    val app = context.applicationContext as EnkoduApp
    val settings = app.settingsStore
    val authStore = app.authStore
    val authRepository = app.authRepository
    val scope = rememberCoroutineScope()
    val existingAuth = remember { authStore.currentAuthConfig() }

    var serverUrl by remember { mutableStateOf(existingAuth.serverUrl) }
    var authMode by remember { mutableStateOf(existingAuth.authMode) }
    var companionToken by remember { mutableStateOf(existingAuth.companionToken) }
    var displayName by remember { mutableStateOf(settings.displayName) }
    var wifiOnlyUploads by remember { mutableStateOf(settings.wifiOnlyUploads) }
    var wifiOnlyDownloads by remember { mutableStateOf(settings.wifiOnlyDownloads) }
    var maxUploadSize by remember { mutableStateOf(settings.maxUploadSizeMb.toString()) }
    var batteryMin by remember { mutableStateOf(settings.batteryMinPercent.toString()) }
    var isTesting by remember { mutableStateOf(false) }
    var probeResult by remember { mutableStateOf<AuthProbeResult?>(null) }

    fun workingConfig(): AuthConfig {
        return AuthConfig(
            serverUrl = serverUrl,
            authMode = authMode,
            companionToken = companionToken
        ).normalized()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") }
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(horizontal = 24.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            Spacer(modifier = Modifier.height(8.dp))

            Text("Authentication", style = MaterialTheme.typography.titleMedium)
            OutlinedTextField(
                value = serverUrl,
                onValueChange = {
                    serverUrl = it
                    probeResult = null
                },
                label = { Text("Server URL") },
                modifier = Modifier.fillMaxWidth(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                isError = probeResult?.state == AuthConnectionState.ServerUnreachable &&
                    !AuthConfigValidator.isValidServerUrl(serverUrl)
            )

            AuthModeSetting(
                title = "Companion token",
                description = "Use bearer auth for protected queue requests.",
                selected = authMode == AuthMode.CompanionToken,
                onSelect = {
                    authMode = AuthMode.CompanionToken
                    probeResult = null
                }
            )
            AuthModeSetting(
                title = "No auth (legacy server)",
                description = "Only use this against a legacy-open server during controlled testing.",
                selected = authMode == AuthMode.None,
                onSelect = {
                    authMode = AuthMode.None
                    probeResult = null
                }
            )

            if (authMode.requiresToken) {
                OutlinedTextField(
                    value = companionToken,
                    onValueChange = {
                        companionToken = it
                        probeResult = null
                    },
                    label = { Text("Companion token") },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                    visualTransformation = PasswordVisualTransformation()
                )
            }

            probeResult?.let { result ->
                Text(
                    text = result.message,
                    color = authStateColor(result.state),
                    style = MaterialTheme.typography.bodyMedium
                )
            }

            Button(
                onClick = {
                    scope.launch {
                        isTesting = true
                        probeResult = authRepository.probe(workingConfig())
                        isTesting = false
                    }
                },
                enabled = !isTesting,
                modifier = Modifier.fillMaxWidth()
            ) {
                if (isTesting) {
                    CircularProgressIndicator(modifier = Modifier.padding(end = 8.dp), strokeWidth = 2.dp)
                    Text("Testing…")
                } else {
                    Text("Test Connection")
                }
            }

            Button(
                onClick = {
                    authStore.clearToken()
                    companionToken = ""
                    probeResult = AuthProbeResult(
                        state = AuthConnectionState.NotConfigured,
                        message = "Companion token cleared."
                    )
                    Toast.makeText(context, "Companion token cleared", Toast.LENGTH_SHORT).show()
                },
                modifier = Modifier.fillMaxWidth()
            ) {
                Text("Clear Token")
            }

            Divider(modifier = Modifier.padding(vertical = 8.dp))

            OutlinedTextField(
                value = displayName,
                onValueChange = { displayName = it },
                label = { Text("Display Name (optional)") },
                modifier = Modifier.fillMaxWidth()
            )

            Divider(modifier = Modifier.padding(vertical = 8.dp))

            Text("Network", style = MaterialTheme.typography.titleMedium)
            SettingsSwitch(
                label = "WiFi-only uploads",
                checked = wifiOnlyUploads,
                onCheckedChange = { wifiOnlyUploads = it }
            )
            SettingsSwitch(
                label = "WiFi-only downloads",
                checked = wifiOnlyDownloads,
                onCheckedChange = { wifiOnlyDownloads = it }
            )
            OutlinedTextField(
                value = maxUploadSize,
                onValueChange = { maxUploadSize = it.filter { c -> c.isDigit() } },
                label = { Text("Max upload size on cellular (MB)") },
                modifier = Modifier.fillMaxWidth(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number)
            )

            Divider(modifier = Modifier.padding(vertical = 8.dp))

            Text("Battery", style = MaterialTheme.typography.titleMedium)
            OutlinedTextField(
                value = batteryMin,
                onValueChange = { batteryMin = it.filter { c -> c.isDigit() } },
                label = { Text("Minimum battery % for transfers") },
                modifier = Modifier.fillMaxWidth(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number)
            )

            Spacer(modifier = Modifier.height(24.dp))

            Button(
                onClick = {
                    val authConfig = workingConfig()
                    if (!AuthConfigValidator.isValidServerUrl(authConfig.serverUrl)) {
                        Toast.makeText(context, "Please enter a valid server URL", Toast.LENGTH_LONG).show()
                        return@Button
                    }
                    if (authConfig.authMode.requiresToken && authConfig.companionToken.isBlank()) {
                        Toast.makeText(context, "Please enter a companion token", Toast.LENGTH_LONG).show()
                        return@Button
                    }
                    authStore.save(authConfig)
                    settings.displayName = displayName.trim()
                    settings.wifiOnlyUploads = wifiOnlyUploads
                    settings.wifiOnlyDownloads = wifiOnlyDownloads
                    settings.maxUploadSizeMb = maxUploadSize.toIntOrNull() ?: 100
                    settings.batteryMinPercent = batteryMin.toIntOrNull() ?: 15
                    Toast.makeText(context, "Settings saved", Toast.LENGTH_SHORT).show()
                    navController.popBackStack()
                },
                modifier = Modifier.fillMaxWidth()
            ) {
                Text("Save")
            }

            Button(
                onClick = { navController.popBackStack() },
                modifier = Modifier.fillMaxWidth()
            ) {
                Text("Cancel")
            }
        }
    }
}

@Composable
private fun AuthModeSetting(
    title: String,
    description: String,
    selected: Boolean,
    onSelect: () -> Unit
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            RadioButton(selected = selected, onClick = onSelect)
            Text(text = title, style = MaterialTheme.typography.bodyLarge)
        }
        Text(
            text = description,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 52.dp)
        )
    }
}

@Composable
private fun SettingsSwitch(
    label: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(label)
        Switch(checked = checked, onCheckedChange = onCheckedChange)
    }
}

private fun authStateColor(state: AuthConnectionState): Color {
    return when (state) {
        AuthConnectionState.Connected -> Color(0xFF2E7D32)
        AuthConnectionState.TokenRejected,
        AuthConnectionState.ServerUnreachable,
        AuthConnectionState.PermissionDenied -> Color(0xFFC62828)
        else -> Color(0xFF616161)
    }
}
