package org.hyperish.serge.window;

import android.app.Activity;
import android.graphics.Rect;
import androidx.core.util.Consumer;
import androidx.window.java.layout.WindowInfoTrackerCallbackAdapter;
import androidx.window.layout.*;
import java.util.ArrayList;

/** Window-relative folding bounds, delivered by Jetpack WindowManager (API 24+). */
public final class WindowPosture implements AutoCloseable {
    private final Activity activity;
    private final WindowInfoTrackerCallbackAdapter tracker;
    private final Consumer<WindowLayoutInfo> listener = this::changed;
    private volatile int[] latest;
    private volatile boolean closed;

    public WindowPosture(Activity activity) {
        this.activity = activity;
        tracker = new WindowInfoTrackerCallbackAdapter(WindowInfoTracker.getOrCreate(activity));
        activity.runOnUiThread(() -> {
            if (!closed) tracker.addWindowLayoutInfoListener(activity, activity::runOnUiThread, listener);
        });
    }
    private void changed(WindowLayoutInfo info) {
        ArrayList<FoldingFeature> folds = new ArrayList<>();
        for (DisplayFeature feature : info.getDisplayFeatures())
            if (feature instanceof FoldingFeature) folds.add((FoldingFeature)feature);
        int[] out = new int[folds.size() * 8];
        int i = 0;
        for (FoldingFeature fold : folds) {
            Rect r = fold.getBounds();
            out[i++] = r.left; out[i++] = r.top; out[i++] = r.right; out[i++] = r.bottom;
            out[i++] = fold.getState() == FoldingFeature.State.HALF_OPENED ? 1 : 0;
            out[i++] = fold.getOrientation() == FoldingFeature.Orientation.VERTICAL ? 1 : 0;
            out[i++] = fold.getOcclusionType() == FoldingFeature.OcclusionType.FULL ? 1 : 0;
            out[i++] = fold.isSeparating() ? 1 : 0;
        }
        if (!closed) latest = out;
    }
    public int[] snapshot() { return latest; }
    public void close() {
        closed = true;
        activity.runOnUiThread(() -> tracker.removeWindowLayoutInfoListener(listener));
    }
}
