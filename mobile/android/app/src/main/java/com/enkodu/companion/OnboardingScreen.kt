package com.enkodu.companion

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
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
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.KeyboardOptions
import com.enkodu.companion.auth.AuthConfig
import com.enkodu.companion.auth.AuthConfigValidator
import com.enkodu.companion.auth.AuthConnectionState
import com.enkodu.companion.auth.AuthMode
import com.enkodu.companion.auth.AuthProbeResult
import kotlinx.coroutines.launch

@Composable
fun OnboardingScreen(onComplete: () -> Unit) {
    var step by remember { mutableStateOf(0) }
    val context = LocalContext.current
    val app = context.applicationContext as EnkoduApp
    val authStore = app.authStore
    val authRepository = app.authRepository
    val existingConfig = remember { authStore.currentAuthConfig() }
    val scope = rememberCoroutineScope()

    var serverUrl by remember { mutableStateOf(existingConfig.serverUrl) }
    var authMode by remember { mutableStateOf(existingConfig.authMode) }
    var companionToken by remember { mutableStateOf(existingConfig.companionToken) }
    var isTesting by remember { mutableStateOf(false) }
    var probeResult by remember { mutableStateOf<AuthProbeResult?>(null) }

    fun currentConfig(): AuthConfig {
        return AuthConfig(
            serverUrl = serverUrl,
            authMode = authMode,
            companionToken = companionToken
        ).normalized()
    }

    fun resetProbe() {
        probeResult = null
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp)
            .verticalScroll(rememberScrollState()),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        when (step) {
            0 -> {
                Text(
                    "Welcome to Enkodu",
                    style = MaterialTheme.typography.headlineLarge,
                    textAlign = TextAlign.Center
                )
                Spacer(modifier = Modifier.height(16.dp))
                Text(
                    "Limited release uses the companion token flow. Connect this device to your queue server before uploads, downloads, or queue status calls.",
                    style = MaterialTheme.typography.bodyLarge,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(modifier = Modifier.height(24.dp))
                Text(
                    "You will need:",
                    style = MaterialTheme.typography.titleMedium,
                    textAlign = TextAlign.Center
                )
                Spacer(modifier = Modifier.height(8.dp))
                Text("1. Your Enkodu server URL", textAlign = TextAlign.Center)
                Text("2. A companion device token from the server", textAlign = TextAlign.Center)
                Text("3. LAN or Tailscale access to the protected /status endpoint", textAlign = TextAlign.Center)
                Spacer(modifier = Modifier.height(32.dp))
                Button(
                    onClick = { step = 1 },
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text("Configure Connection")
                }
            }

            1 -> {
                Text(
                    "Authenticate Device",
                    style = MaterialTheme.typography.headlineMedium,
                    textAlign = TextAlign.Center
                )
                Spacer(modifier = Modifier.height(16.dp))

                OutlinedTextField(
                    value = serverUrl,
                    onValueChange = {
                        serverUrl = it
                        resetProbe()
                    },
                    label = { Text("Server URL") },
                    modifier = Modifier.fillMaxWidth(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    isError = probeResult?.state == AuthConnectionState.ServerUnreachable &&
                        !AuthConfigValidator.isValidServerUrl(serverUrl)
                )

                Spacer(modifier = Modifier.height(16.dp))
                Text(
                    "Auth Mode",
                    style = MaterialTheme.typography.titleSmall,
                    modifier = Modifier.fillMaxWidth()
                )
                AuthModeOption(
                    title = "Companion token",
                    description = "Recommended for this release. Sends a bearer token on protected queue requests.",
                    selected = authMode == AuthMode.CompanionToken,
                    onSelect = {
                        authMode = AuthMode.CompanionToken
                        resetProbe()
                    }
                )
                AuthModeOption(
                    title = "No auth (legacy server)",
                    description = "Only use this if the server is intentionally running with legacy machine access.",
                    selected = authMode == AuthMode.None,
                    onSelect = {
                        authMode = AuthMode.None
                        resetProbe()
                    }
                )

                if (authMode.requiresToken) {
                    Spacer(modifier = Modifier.height(16.dp))
                    OutlinedTextField(
                        value = companionToken,
                        onValueChange = {
                            companionToken = it
                            resetProbe()
                        },
                        label = { Text("Companion token") },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                        visualTransformation = PasswordVisualTransformation()
                    )
                }

                probeResult?.let { result ->
                    Spacer(modifier = Modifier.height(12.dp))
                    Text(
                        text = result.message,
                        color = authStateColor(result.state),
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.fillMaxWidth()
                    )
                }

                Spacer(modifier = Modifier.height(16.dp))
                Button(
                    onClick = {
                        isTesting = true
                        scope.launch {
                            probeResult = authRepository.probe(currentConfig())
                            isTesting = false
                        }
                    },
                    enabled = !isTesting,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    if (isTesting) {
                        CircularProgressIndicator(
                            modifier = Modifier.padding(end = 8.dp),
                            strokeWidth = 2.dp
                        )
                        Text("Testing…")
                    } else {
                        Text("Test Connection")
                    }
                }

                Spacer(modifier = Modifier.height(24.dp))
                Button(
                    onClick = {
                        val saved = authStore.save(currentConfig())
                        if (probeResult?.state == AuthConnectionState.Connected && saved.isConfigured()) {
                            onComplete()
                        }
                    },
                    enabled = probeResult?.state == AuthConnectionState.Connected,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text("Continue")
                }
            }
        }
    }
}

@Composable
private fun AuthModeOption(
    title: String,
    description: String,
    selected: Boolean,
    onSelect: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 8.dp)
    ) {
        androidx.compose.foundation.layout.Row(
            verticalAlignment = Alignment.CenterVertically
        ) {
            RadioButton(
                selected = selected,
                onClick = onSelect
            )
            Text(
                text = title,
                style = MaterialTheme.typography.bodyLarge
            )
        }
        Text(
            text = description,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 52.dp)
        )
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
