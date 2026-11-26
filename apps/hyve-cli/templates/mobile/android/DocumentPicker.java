package org.hyperish.serge.documents;

import android.app.Activity;
import android.app.Fragment;
import android.app.FragmentManager;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;

/** Routes Storage Access Framework results to the NativeActivity without replacing it. */
@SuppressWarnings("deprecation")
public final class DocumentPicker extends Fragment {
    private static final int PICK = 1;
    private Ticket ticket;

    public static final class Ticket {
        private volatile int status;
        private String value;
        private DocumentPicker owner;

        private synchronized void finish(int result, String text) {
            if (status == 0) {
                value = text;
                status = result;
            }
        }
        public int status() { return status; }
        public synchronized String value() { return value; }
        public void cancel() {
            finish(1, null);
            new Handler(Looper.getMainLooper()).post(() -> {
                if (owner != null) owner.removePicker();
            });
        }
    }

    public static Ticket pick(Activity activity, String mime, String suggestedName) {
        Ticket request = new Ticket();
        activity.runOnUiThread(() -> {
            if (request.status() != 0) return;
            try {
                FragmentManager manager = activity.getFragmentManager();
                DocumentPicker picker = new DocumentPicker();
                picker.ticket = request;
                request.owner = picker;
                manager.beginTransaction().add(picker, null).commitNow();
                Intent intent = new Intent(suggestedName == null ? Intent.ACTION_OPEN_DOCUMENT : Intent.ACTION_CREATE_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType(mime);
                if (suggestedName != null) intent.putExtra(Intent.EXTRA_TITLE, suggestedName);
                picker.startActivityForResult(intent, PICK);
            } catch (RuntimeException error) {
                request.finish(3, error.toString());
                if (request.owner != null) request.owner.removePicker();
            }
        });
        return request;
    }

    @Override public void onCreate(Bundle savedState) {
        super.onCreate(savedState);
        setRetainInstance(true);
        // A restored process has no Rust caller for the previous request.
        if (ticket == null) removePicker();
    }

    @Override public void onActivityResult(int code, int result, Intent data) {
        super.onActivityResult(code, result, data);
        if (code != PICK || ticket == null) return;
        Uri uri = data == null ? null : data.getData();
        if (result != Activity.RESULT_OK) ticket.finish(1, null);
        else if (uri == null || !"content".equals(uri.getScheme())) ticket.finish(3, "no document content URI");
        else ticket.finish(2, uri.toString());
        removePicker();
    }

    private void removePicker() {
        if (ticket != null) ticket.owner = null;
        FragmentManager manager = getFragmentManager();
        if (manager != null && !manager.isDestroyed() && !isRemoving()) {
            manager.beginTransaction().remove(this).commitAllowingStateLoss();
        }
    }
    @Override public void onDestroy() {
        if (ticket != null) {
            ticket.finish(1, null);
            ticket.owner = null;
        }
        super.onDestroy();
    }
}
