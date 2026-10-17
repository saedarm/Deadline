package com.saedarm.deadline.reminders

import android.Manifest
import android.app.Activity
import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.TimeUnit

private const val PREFS = "deadline_reminders"
private const val KEY_LIST = "reminders"
private const val KEY_URL = "server_url"
private const val KEY_TOKEN = "token"
private const val CHANNEL = "deadline"

/**
 * Called from Rust (see src/reminders.rs). The constructor signature (Activity) is
 * what the Dioxus FFI layer expects.
 */
class ReminderPlugin(private val activity: Activity) {
    private val ctx: Context get() = activity.applicationContext

    /** Remember the server so the background sync can fetch reminders on its own. */
    fun configure(serverUrl: String, token: String): String {
        prefs(ctx).edit().putString(KEY_URL, serverUrl).putString(KEY_TOKEN, token).apply()
        val work = PeriodicWorkRequestBuilder<SyncWorker>(30, TimeUnit.MINUTES)
            .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
            .build()
        WorkManager.getInstance(ctx)
            .enqueueUniquePeriodicWork("deadline-sync", ExistingPeriodicWorkPolicy.KEEP, work)
        return "ok"
    }

    /** Replace all scheduled reminders with this JSON list. Returns the count scheduled. */
    fun schedule(json: String): String = Alarms.replace(ctx, json).toString()

    fun status(): String = JSONObject()
        .put("notifications", NotificationManagerCompat.from(ctx).areNotificationsEnabled())
        .put("exactAlarms", Alarms.canExact(ctx))
        .put("unrestrictedBattery", batteryUnrestricted())
        .toString()

    fun requestPermissions(): String {
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(ctx, Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED
        ) {
            activity.runOnUiThread {
                ActivityCompat.requestPermissions(
                    activity, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 4242
                )
            }
        }
        return status()
    }

    /** Samsung puts apps to sleep aggressively. This opens the "allow background" prompt. */
    fun openBatterySettings(): String {
        activity.runOnUiThread {
            try {
                activity.startActivity(
                    Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
                        .setData(Uri.parse("package:${ctx.packageName}"))
                )
            } catch (_: Exception) {
                activity.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
            }
        }
        return "ok"
    }

    private fun batteryUnrestricted(): Boolean {
        val pm = ctx.getSystemService(Context.POWER_SERVICE) as PowerManager
        return pm.isIgnoringBatteryOptimizations(ctx.packageName)
    }
}

private fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

/** Alarm bookkeeping shared by the plugin, the re-arm receiver and the background sync. */
object Alarms {
    fun canExact(ctx: Context): Boolean {
        if (Build.VERSION.SDK_INT < 31) return true
        val am = ctx.getSystemService(Context.ALARM_SERVICE) as AlarmManager
        return am.canScheduleExactAlarms()
    }

    /** Cancel what's scheduled, save the new list, schedule it. */
    fun replace(ctx: Context, json: String): Int {
        val old = load(ctx)
        val am = ctx.getSystemService(Context.ALARM_SERVICE) as AlarmManager
        for (i in 0 until old.length()) {
            am.cancel(pending(ctx, old.getJSONObject(i)))
        }
        prefs(ctx).edit().putString(KEY_LIST, json).apply()
        return rearm(ctx)
    }

    /** Schedule everything saved that's still in the future. */
    fun rearm(ctx: Context): Int {
        val list = load(ctx)
        val am = ctx.getSystemService(Context.ALARM_SERVICE) as AlarmManager
        val now = System.currentTimeMillis()
        var count = 0
        for (i in 0 until list.length()) {
            val r = list.getJSONObject(i)
            val at = r.getLong("at_ms")
            if (at <= now) continue
            val pi = pending(ctx, r)
            if (canExact(ctx)) {
                am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
            } else {
                am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
            }
            count++
        }
        return count
    }

    private fun load(ctx: Context): JSONArray =
        try { JSONArray(prefs(ctx).getString(KEY_LIST, "[]")) } catch (_: Exception) { JSONArray() }

    private fun pending(ctx: Context, r: JSONObject): PendingIntent {
        val id = r.getLong("id")
        val intent = Intent(ctx, ReminderReceiver::class.java)
            .putExtra("id", id)
            .putExtra("title", r.optString("title"))
            .putExtra("body", r.optString("body"))
        return PendingIntent.getBroadcast(
            ctx, id.toInt(), intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
    }
}

/** Fires when an alarm goes off and posts the notification. */
class ReminderReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (nm.getNotificationChannel(CHANNEL) == null) {
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL, "Deadlines", NotificationManager.IMPORTANCE_HIGH)
                    .apply { description = "Task reminders and the morning edition" }
            )
        }
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(ctx, Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED
        ) return

        val open = ctx.packageManager.getLaunchIntentForPackage(ctx.packageName)?.let {
            PendingIntent.getActivity(ctx, 0, it, PendingIntent.FLAG_IMMUTABLE)
        }
        val n = NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(R.drawable.ic_deadline)
            .setContentTitle(intent.getStringExtra("title") ?: "Deadline")
            .setContentText(intent.getStringExtra("body") ?: "")
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_REMINDER)
            .setAutoCancel(true)
            .setContentIntent(open)
            .build()
        NotificationManagerCompat.from(ctx).notify(intent.getLongExtra("id", 0).toInt(), n)
    }
}

/** Reboots, app updates and clock changes clear alarms. Put them back. */
class RearmReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        Alarms.rearm(ctx)
    }
}

/** Every ~30 minutes, pull the latest reminders so tasks added on the desktop still buzz. */
class SyncWorker(ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result {
        val p = prefs(applicationContext)
        val base = p.getString(KEY_URL, null) ?: return Result.success()
        val token = p.getString(KEY_TOKEN, "") ?: ""
        return try {
            val conn = URL("${base.trimEnd('/')}/api/reminders").openConnection() as HttpURLConnection
            conn.connectTimeout = 15_000
            conn.readTimeout = 15_000
            conn.setRequestProperty("Authorization", "Bearer $token")
            if (conn.responseCode != 200) return Result.retry()
            val body = conn.inputStream.bufferedReader().use { it.readText() }
            JSONArray(body) // validate before replacing what's scheduled
            Alarms.replace(applicationContext, body)
            Result.success()
        } catch (_: Exception) {
            Result.retry()
        }
    }
}
