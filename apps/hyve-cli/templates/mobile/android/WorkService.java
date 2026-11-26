package org.hyperish.serge.background;

import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.SystemClock;

/** Completes user-visible, bounded work that the app started while in the foreground. */
public final class WorkService extends Service {
    private static final String CHANNEL = "org.hyperish.serge.background";
    private static final int NOTIFICATION = 0x534752;
    private static final long MAX_MILLIS = 165_000;
    private static final Handler MAIN = new Handler(Looper.getMainLooper());
    private static Batch current;
    private static long generation;
    private Batch promoted;
    private int startId;

    private static final class Batch {
        final long id = ++generation;
        final long deadline = SystemClock.elapsedRealtime() + MAX_MILLIS;
        int count;
        WorkService service;
    }

    public static final class Lease implements Runnable {
        private final Batch batch;
        private final long deadline;
        private boolean closed;

        private Lease(Batch batch, long millis) {
            this.batch = batch;
            deadline = Math.min(batch.deadline, SystemClock.elapsedRealtime() + millis);
            batch.count++;
            MAIN.postDelayed(this, Math.max(0, deadline - SystemClock.elapsedRealtime()));
        }

        public int status() {
            synchronized (WorkService.class) {
                if (SystemClock.elapsedRealtime() >= deadline) close();
                if (closed || current != batch) return 2;
                return batch.service == null ? 0 : 1;
            }
        }

        @Override public void run() { close(); }

        public void close() {
            synchronized (WorkService.class) {
                if (closed) return;
                closed = true;
                MAIN.removeCallbacks(this);
                if (--batch.count == 0 && current == batch) {
                    current = null;
                    if (batch.service != null) batch.service.finish();
                }
            }
        }
    }

    public static synchronized Lease begin(Activity activity, String title, String text, long millis) {
        if (millis < 1 || millis > MAX_MILLIS) throw new IllegalArgumentException("invalid work duration");
        if (current == null) {
            Batch batch = new Batch();
            Intent command = new Intent(activity, WorkService.class)
                .putExtra("generation", batch.id).putExtra("title", title).putExtra("text", text);
            activity.startForegroundService(command);
            current = batch;
        }
        return new Lease(current, millis);
    }

    private Notification notification(Intent command) {
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel(
            CHANNEL, getApplicationInfo().loadLabel(getPackageManager()), NotificationManager.IMPORTANCE_LOW));
        Notification.Builder note = new Notification.Builder(this, CHANNEL)
            .setContentTitle(command.getStringExtra("title")).setContentText(command.getStringExtra("text"))
            .setSmallIcon(android.R.drawable.stat_notify_sync).setOngoing(true).setOnlyAlertOnce(true);
        if (Build.VERSION.SDK_INT >= 31) note.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE);
        return note.build();
    }

    @Override public int onStartCommand(Intent command, int flags, int id) {
        synchronized (WorkService.class) {
            startId = id;
            if (command == null) {
                finish();
                return START_NOT_STICKY;
            }
            // Even a lease dropped before delivery must satisfy startForegroundService's handshake.
            try {
                Notification note = notification(command);
                if (Build.VERSION.SDK_INT >= 34) {
                    startForeground(NOTIFICATION, note, ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE);
                } else {
                    startForeground(NOTIFICATION, note);
                }
            } catch (RuntimeException e) {
                android.util.Log.w("Serge", "Foreground work declined", e);
                if (current != null && current.id == command.getLongExtra("generation", 0)) current = null;
                finish();
                return START_NOT_STICKY;
            }
            if (current != null && current.id == command.getLongExtra("generation", 0)) {
                promoted = current;
                promoted.service = this;
            } else {
                finish();
            }
            return START_NOT_STICKY;
        }
    }

    private void finish() {
        if (stopSelfResult(startId)) stopForeground(STOP_FOREGROUND_REMOVE);
    }

    @Override public void onTimeout(int id) {
        synchronized (WorkService.class) {
            if (id == startId) {
                if (promoted == current) current = null;
                finish();
            }
        }
    }

    @Override public void onTimeout(int id, int type) { onTimeout(id); }

    @Override public void onDestroy() {
        synchronized (WorkService.class) {
            if (promoted != null) {
                if (current == promoted) current = null;
                promoted.service = null;
            }
        }
        super.onDestroy();
    }

    @Override public IBinder onBind(Intent intent) { return null; }
}
