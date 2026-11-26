package org.hyperish.serge.notifications;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.database.sqlite.SQLiteDatabase;
import android.database.sqlite.SQLiteOpenHelper;
import android.net.Uri;
import java.util.List;
import java.util.UUID;

/** Explicit notification actions. The framework can deliver a selection after process restart. */
public final class Actions extends BroadcastReceiver {
    private static final String CHANNEL = "serge.local";
    private static final String TAG = "org.hyperish.serge.local.";
    private static Store store;

    private static final class Store extends SQLiteOpenHelper {
        Store(Context context) { super(context.getApplicationContext(), "serge-notification-actions.db", null, 1); }
        @Override public void onCreate(SQLiteDatabase db) {
            db.execSQL("CREATE TABLE pending (id TEXT PRIMARY KEY, revision TEXT NOT NULL)");
            db.execSQL("CREATE TABLE responses (seq INTEGER PRIMARY KEY, id TEXT NOT NULL, action TEXT NOT NULL)");
        }
        @Override public void onUpgrade(SQLiteDatabase db, int from, int to) {
            throw new IllegalStateException("unknown notification store version");
        }
    }
    private static synchronized SQLiteDatabase db(Context context) {
        if (store == null) store = new Store(context);
        return store.getWritableDatabase();
    }
    private static NotificationManager manager(Context context) {
        return context.getSystemService(NotificationManager.class);
    }
    private static PendingIntent token(Context context, String id, String revision, String action) {
        Uri data = new Uri.Builder().scheme("serge-notification").authority(context.getPackageName())
            .appendPath(id).appendPath(revision).appendPath(action).build();
        Intent intent = new Intent(context, Actions.class).setData(data);
        return PendingIntent.getBroadcast(context, 0, intent, PendingIntent.FLAG_IMMUTABLE);
    }

    public static boolean post(Context context, String id, String title, String text,
                               String[] keys, String[] labels) {
        NotificationManager nm = manager(context);
        if (!nm.areNotificationsEnabled()) return false;
        String label = context.getApplicationInfo().loadLabel(context.getPackageManager()).toString();
        nm.createNotificationChannel(new NotificationChannel(CHANNEL, label, NotificationManager.IMPORTANCE_DEFAULT));
        if (nm.getNotificationChannel(CHANNEL).getImportance() == NotificationManager.IMPORTANCE_NONE) return false;
        String revision = UUID.randomUUID().toString();
        int icon = context.getApplicationInfo().icon;
        Notification.Builder builder = new Notification.Builder(context, CHANNEL)
            .setSmallIcon(icon == 0 ? android.R.drawable.ic_dialog_info : icon)
            .setContentTitle(title).setContentText(text).setOnlyAlertOnce(true)
            .setDeleteIntent(token(context, id, revision, "~dismiss"));
        for (int i = 0; i < keys.length; i++) {
            builder.addAction(new Notification.Action.Builder(null, labels[i], token(context, id, revision, keys[i])).build());
        }
        SQLiteDatabase db = db(context);
        db.beginTransaction();
        try {
            db.execSQL("INSERT OR REPLACE INTO pending VALUES (?,?)", new Object[]{id,revision});
            nm.notify(TAG + id, 1, builder.build());
            db.setTransactionSuccessful();
        } finally { db.endTransaction(); }
        return true;
    }
    public static void cancel(Context context, String id) {
        SQLiteDatabase db = db(context);
        db.beginTransaction();
        try {
            db.delete("pending", "id=?", new String[]{id});
            manager(context).cancel(TAG + id, 1);
            db.setTransactionSuccessful();
        } finally { db.endTransaction(); }
    }
    @Override public void onReceive(Context context, Intent intent) {
        Uri uri = intent.getData();
        if (uri == null || !"serge-notification".equals(uri.getScheme()) || !context.getPackageName().equals(uri.getAuthority())) return;
        List<String> fields = uri.getPathSegments();
        if (fields.size() != 3) return;
        String id = fields.get(0), action = fields.get(2);
        SQLiteDatabase db = db(context);
        db.beginTransaction();
        try {
            if (db.delete("pending", "id=? AND revision=?", new String[]{id,fields.get(1)}) == 1) {
                if (!"~dismiss".equals(action)) {
                    db.execSQL("INSERT INTO responses(id,action) VALUES (?,?)", new Object[]{id,action});
                    manager(context).cancel(TAG + id, 1);
                }
            }
            db.setTransactionSuccessful();
        } finally { db.endTransaction(); }
    }
    public static String[] take(Context context) {
        SQLiteDatabase db = db(context);
        db.beginTransaction();
        try (Cursor c = db.rawQuery("SELECT seq,id,action FROM responses ORDER BY seq LIMIT 1", null)) {
            if (!c.moveToFirst()) return null;
            String[] result = {c.getString(1), c.getString(2)};
            db.delete("responses", "seq=?", new String[]{c.getString(0)});
            db.setTransactionSuccessful();
            return result;
        } finally { db.endTransaction(); }
    }
}
