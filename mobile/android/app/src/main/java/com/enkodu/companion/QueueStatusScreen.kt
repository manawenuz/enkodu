package com.enkodu.companion

import android.widget.Toast
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshContainer
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.navigation.NavHostController
import com.enkodu.companion.auth.AuthProbeResultMapper
import com.enkodu.companion.api.StatusResponse
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QueueStatusScreen(navController: NavHostController) {
    val context = LocalContext.current
    val app = context.applicationContext as EnkoduApp
    val authStore = app.authStore
    val authRepository = app.authRepository
    val authConfig by authStore.authConfig.collectAsState()
    var status by remember { mutableStateOf<StatusResponse?>(null) }
    var isLoading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val pullRefreshState = rememberPullToRefreshState()

    val refreshData = {
        if (!authConfig.isConfigured()) {
            error = "Configure the server URL and companion token before loading queue status."
            status = null
        } else {
            isLoading = true
            error = null
            CoroutineScope(Dispatchers.IO).launch {
                try {
                    val api = authRepository.createApi(authConfig.serverUrl)
                    val resp = api.getStatus()
                    if (resp.isSuccessful) {
                        status = resp.body()
                    } else {
                        error = AuthProbeResultMapper.fromHttpCode(resp.code()).message
                        status = null
                    }
                } catch (e: Exception) {
                    error = e.message ?: "Unable to load queue status"
                    status = null
                } finally {
                    isLoading = false
                }
            }
        }
    }

    LaunchedEffect(authConfig.serverUrl, authConfig.authMode, authConfig.companionToken) {
        refreshData()
    }

    if (pullRefreshState.isRefreshing) {
        LaunchedEffect(true) {
            refreshData()
            pullRefreshState.endRefresh()
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Queue Status") }
            )
        }
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .nestedScroll(pullRefreshState.nestedScrollConnection)
        ) {
            when {
                isLoading && status == null -> {
                    Column(
                        modifier = Modifier.fillMaxSize(),
                        verticalArrangement = Arrangement.Center,
                        horizontalAlignment = Alignment.CenterHorizontally
                    ) {
                        CircularProgressIndicator()
                        Spacer(modifier = Modifier.height(16.dp))
                        Text("Loading...")
                    }
                }
                error != null && status == null -> {
                    Column(
                        modifier = Modifier.fillMaxSize(),
                        verticalArrangement = Arrangement.Center,
                        horizontalAlignment = Alignment.CenterHorizontally
                    ) {
                        Text(
                            error!!,
                            color = Color(0xFFC62828),
                            style = MaterialTheme.typography.bodyMedium
                        )
                        Spacer(modifier = Modifier.height(16.dp))
                        Button(onClick = { refreshData() }) {
                            Text("Retry")
                        }
                    }
                }
                status != null -> {
                    LazyColumn(
                        modifier = Modifier
                            .fillMaxSize()
                            .padding(16.dp)
                    ) {
                        item {
                            StatusCard(status!!)
                            Spacer(modifier = Modifier.height(16.dp))
                        }
                        item {
                            Text(
                                "Active Jobs",
                                style = MaterialTheme.typography.titleMedium,
                                fontWeight = FontWeight.SemiBold
                            )
                            Spacer(modifier = Modifier.height(8.dp))
                        }
                        item {
                            Text(
                                "Active job details would appear here",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        item {
                            Spacer(modifier = Modifier.height(24.dp))
                            Button(
                                onClick = { navController.popBackStack() },
                                modifier = Modifier.fillMaxWidth()
                            ) {
                                Text("Back")
                            }
                        }
                    }
                }
            }
            PullToRefreshContainer(
                state = pullRefreshState,
                modifier = Modifier.align(Alignment.TopCenter)
            )
        }
    }
}

@Composable
private fun StatusCard(status: StatusResponse) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant
        )
    ) {
        Column(
            modifier = Modifier.padding(16.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceEvenly
            ) {
                StatusItem("Pending", status.pending, Color(0xFFFFA726))
                StatusItem("Active", status.active, Color(0xFF42A5F5))
                StatusItem("Done", status.done, Color(0xFF66BB6A))
                StatusItem("Failed", status.failed, Color(0xFFEF5350))
            }
        }
    }
}

@Composable
private fun StatusItem(label: String, value: Long, color: Color) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(
            value.toString(),
            style = MaterialTheme.typography.headlineMedium,
            color = color,
            fontWeight = FontWeight.Bold
        )
        Text(
            label,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}
