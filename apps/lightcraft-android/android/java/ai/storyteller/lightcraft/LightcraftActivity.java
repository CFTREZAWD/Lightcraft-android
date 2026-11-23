// LightCraft for Android: the activity that hosts the native app (liblightcraft_android.so).
//
// Everything LightCraft does runs in Rust (android_main). This class only adds what a plain
// NativeActivity can't do from native code: the system file picker (its results come back to an
// activity), the soft keyboard, opening links, and asking for the media permission. Every
// method is called from native code through JNI and hops onto the UI thread itself.
package ai.storyteller.lightcraft;

import android.Manifest;
import android.app.NativeActivity;
import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.view.View;
import android.view.WindowManager;
import android.view.inputmethod.InputMethodManager;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;

public class LightcraftActivity extends NativeActivity {
    /** Request codes above this one are pick requests: `PICK_BASE + id`. */
    private static final int PICK_BASE = 0x4c00;
    private static final int PERMISSION_REQUEST = 0x4b01;

    static {
        System.loadLibrary("lightcraft_android");
    }

    /** The chosen files were copied to `paths` (empty: cancelled or nothing could be read). */
    private static native void nativePicked(int id, String[] paths, String error);

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
    }

    /**
     * Show the system picker. `mimes` are the types offered (`image/*` for photos), `multiple`
     * allows several files; each is copied into `destDir` and the copies' paths are reported.
     */
    public void pick(final int id, final String[] mimes, final boolean multiple, final String destDir) {
        runOnUiThread(new Runnable() { @Override public void run() {
            try {
                Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType(mimes.length == 1 ? mimes[0] : "*/*");
                if (mimes.length > 1) {
                    intent.putExtra(Intent.EXTRA_MIME_TYPES, mimes);
                }
                intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple);
                pendingDest = destDir;
                startActivityForResult(intent, PICK_BASE + id);
            } catch (Exception e) {
                nativePicked(id, new String[0], "The file picker couldn't open: " + e);
            }
        }});
    }

    private String pendingDest = null;

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode < PICK_BASE) {
            return;
        }
        final int id = requestCode - PICK_BASE;
        final ArrayList<Uri> uris = new ArrayList<>();
        if (resultCode == RESULT_OK && data != null) {
            ClipData clip = data.getClipData();
            if (clip != null) {
                for (int i = 0; i < clip.getItemCount(); i++) {
                    uris.add(clip.getItemAt(i).getUri());
                }
            } else if (data.getData() != null) {
                uris.add(data.getData());
            }
        }
        final String dest = pendingDest;
        // copying a large raw takes a moment: off the UI thread
        new Thread(new Runnable() { @Override public void run() {
            ArrayList<String> paths = new ArrayList<>();
            String error = null;
            for (Uri uri : uris) {
                try {
                    paths.add(copyIn(uri, dest));
                } catch (Exception e) {
                    error = "A chosen file couldn't be read: " + e.getMessage();
                }
            }
            nativePicked(id, paths.toArray(new String[0]), error);
        }}, "lc-pick-copy").start();
    }

    /** Copy `uri` into `destDir` under its display name (never replacing a file). */
    private String copyIn(Uri uri, String destDir) throws Exception {
        String name = displayName(uri);
        File dir = new File(destDir);
        dir.mkdirs();
        File out = uniqueFile(dir, name);
        try (InputStream in = getContentResolver().openInputStream(uri);
             OutputStream os = new FileOutputStream(out)) {
            if (in == null) {
                throw new Exception("no data");
            }
            byte[] buf = new byte[1 << 16];
            int n;
            while ((n = in.read(buf)) > 0) {
                os.write(buf, 0, n);
            }
        } catch (Exception e) {
            out.delete();
            throw e;
        }
        return out.getAbsolutePath();
    }

    private String displayName(Uri uri) {
        String name = null;
        try (Cursor c = getContentResolver().query(uri, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst()) {
                name = c.getString(0);
            }
        } catch (Exception ignored) {
        }
        if (name == null || name.isEmpty()) {
            name = uri.getLastPathSegment();
        }
        if (name == null || name.isEmpty()) {
            name = "photo";
        }
        return name.replace('/', '_');
    }

    private static File uniqueFile(File dir, String name) {
        File f = new File(dir, name);
        if (!f.exists()) {
            return f;
        }
        int dot = name.lastIndexOf('.');
        String stem = dot > 0 ? name.substring(0, dot) : name;
        String ext = dot > 0 ? name.substring(dot) : "";
        for (int n = 2; ; n++) {
            f = new File(dir, stem + " (" + n + ")" + ext);
            if (!f.exists()) {
                return f;
            }
        }
    }

    /** Show or hide the soft keyboard (a text field took or lost the focus). */
    public void showKeyboard(final boolean show) {
        runOnUiThread(new Runnable() { @Override public void run() {
            InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
            View view = getWindow().getDecorView();
            if (imm == null || view == null) {
                return;
            }
            if (show) {
                view.setFocusable(true);
                view.setFocusableInTouchMode(true);
                view.requestFocus();
                imm.showSoftInput(view, InputMethodManager.SHOW_IMPLICIT);
            } else {
                imm.hideSoftInputFromWindow(view.getWindowToken(), 0);
            }
        }});
    }

    /** Open an https link in the browser. */
    public void openUrl(final String url) {
        runOnUiThread(new Runnable() { @Override public void run() {
            try {
                Intent intent = new Intent(Intent.ACTION_VIEW, Uri.parse(url));
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                startActivity(intent);
            } catch (Exception ignored) {
            }
        }});
    }

    /** Whether the photos in shared storage can be listed (the Local panel's Camera and Pictures). */
    public boolean hasMediaAccess() {
        return checkSelfPermission(mediaPermission()) == PackageManager.PERMISSION_GRANTED;
    }

    /** Ask for the media permission (once; Android remembers a refusal). */
    public void requestMediaAccess() {
        runOnUiThread(new Runnable() { @Override public void run() {
            if (!hasMediaAccess()) {
                requestPermissions(new String[]{mediaPermission()}, PERMISSION_REQUEST);
            }
        }});
    }

    private static String mediaPermission() {
        return Build.VERSION.SDK_INT >= 33 ? Manifest.permission.READ_MEDIA_IMAGES : Manifest.permission.READ_EXTERNAL_STORAGE;
    }
}
