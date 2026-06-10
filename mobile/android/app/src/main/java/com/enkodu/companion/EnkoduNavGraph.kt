package com.enkodu.companion

import androidx.compose.runtime.Composable
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController

@Composable
fun EnkoduNavGraph() {
    val navController = rememberNavController()

    NavHost(navController = navController, startDestination = "home") {
        composable("home") {
            // Placeholder for home screen
            // Will show status or AV1 gate depending on capability
            HomeScreen(navController)
        }
        composable("queue") {
            QueueStatusScreen(navController)
        }
        composable("settings") {
            SettingsScreen(navController)
        }
    }
}
