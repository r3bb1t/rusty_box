package com.rustybox.android;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.os.PowerManager;

/**
 * Keeps Rusty Box running while a VM runs.
 *
 * An app that runs only an activity is one Android may put to sleep and
 * close once the screen has been off for a while; the VMs it runs go with
 * it. A foreground service tells Android that the user waits on work they
 * started, and its partial wake lock keeps the processor running with the
 * screen off. The native shell starts this service when the first VM powers
 * on and stops it once the last one is off (rusty_box_gui src/android.rs,
 * {@code AliveEffect::Phone}).
 */
public final class VmService extends Service {
    private static final String CHANNEL_ID = "running_vms";
    private static final int NOTIFICATION_ID = 1;
    private static final String WAKE_LOCK_TAG = "RustyBox:running-vms";
    /** The activity the notification brings back. */
    private static final String ACTIVITY = "android.app.NativeActivity";

    private PowerManager.WakeLock wakeLock;

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        Notification notification = notification();
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(
                    NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        } else {
            startForeground(NOTIFICATION_ID, notification);
        }
        if (wakeLock == null) {
            PowerManager power = (PowerManager) getSystemService(Context.POWER_SERVICE);
            wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_LOCK_TAG);
            wakeLock.setReferenceCounted(false);
            wakeLock.acquire();
        }
        // A process Android ends takes its VMs with it: there is nothing for
        // a restarted service to keep running.
        return START_NOT_STICKY;
    }

    @Override
    public void onDestroy() {
        if (wakeLock != null) {
            wakeLock.release();
            wakeLock = null;
        }
        super.onDestroy();
    }

    /** The notice a foreground service must show: what runs, and a way back to it. */
    @SuppressWarnings("deprecation")
    private Notification notification() {
        Notification.Builder builder;
        if (Build.VERSION.SDK_INT >= 26) {
            NotificationManager manager =
                    (NotificationManager) getSystemService(Context.NOTIFICATION_SERVICE);
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID, "Running VMs", NotificationManager.IMPORTANCE_LOW);
            channel.setDescription(
                    "Shown while a VM runs, so that it keeps running with the screen off.");
            manager.createNotificationChannel(channel);
            builder = new Notification.Builder(this, CHANNEL_ID);
        } else {
            builder = new Notification.Builder(this);
        }
        // The task the app already has is brought to the front: the activity is
        // singleTask, so no second instance is made.
        Intent open = new Intent(Intent.ACTION_MAIN)
                .addCategory(Intent.CATEGORY_LAUNCHER)
                .setClassName(this, ACTIVITY)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        PendingIntent tap = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE);
        return builder
                .setSmallIcon(android.R.drawable.ic_media_play)
                .setContentTitle("Rusty Box")
                .setContentText("A VM is running. Tap to return to it.")
                .setContentIntent(tap)
                .setOngoing(true)
                .build();
    }
}
