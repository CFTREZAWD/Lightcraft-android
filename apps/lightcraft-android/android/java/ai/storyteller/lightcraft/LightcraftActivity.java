// LightCraft for Android: the activity that hosts the native app (liblightcraft_android.so).
//
// Everything LightCraft does runs in Rust (android_main). This class only adds what a plain
// NativeActivity can't do from native code: the system file picker (its results come back to an
// activity), the soft keyboard, opening links, and asking for the media permission. Every
// method is called from native code through JNI and hops onto the UI thread itself.
package ai.storyteller.lightcraft;

import android.Manifest;
import android.app.NativeActivity;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.IntentFilter;
import android.content.pm.PackageInstaller;
import android.provider.Settings;
import java.io.FileInputStream;
import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.ext.SdkExtensions;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
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

    /** The package installer's answer to `installApk` (`ok` false: failed or cancelled, with why). */
    private static native void nativeInstallStatus(boolean ok, String message);

    private static final String INSTALL_ACTION = "ai.storyteller.lightcraft.INSTALL_STATUS";
    private BroadcastReceiver installReceiver;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        hideSystemBars();
        installReceiver = new BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent intent) {
                int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
                if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                    // Android's confirmation screen
                    Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                    if (confirm != null) {
                        confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                        startActivity(confirm);
                    }
                } else if (status == PackageInstaller.STATUS_SUCCESS) {
                    nativeInstallStatus(true, "");
                } else {
                    String msg = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE);
                    if (status == PackageInstaller.STATUS_FAILURE_ABORTED) {
                        // also what Samsung's Auto Blocker answers: it blocks every app install outside the stores
                        msg = "it was cancelled. On a Samsung device, Auto Blocker stops app installs outside the stores: "
                                + "turn it off (Settings > Security and privacy > Auto Blocker), update, then turn it back on if you like";
                    } else if (status == PackageInstaller.STATUS_FAILURE_CONFLICT || status == PackageInstaller.STATUS_FAILURE_INCOMPATIBLE) {
                        msg = "it is signed with another key than the installed app (uninstall this one first, then install the new one from its release page)";
                    }
                    nativeInstallStatus(false, msg == null ? ("status " + status) : msg);
                }
            }
        };
        IntentFilter filter = new IntentFilter(INSTALL_ACTION);
        if (Build.VERSION.SDK_INT >= 33) {
            registerReceiver(installReceiver, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            registerReceiver(installReceiver, filter);
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        hideSystemBars();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        // the keyboard, a dialog or the notification shade bring the bars back: hide them again
        if (hasFocus) {
            hideSystemBars();
        }
    }

    /**
     * Full screen: the status bar (clock, notifications) and the navigation bar would otherwise
     * cover the top bar's menus and the panels at the edges. A swipe from an edge shows them for a
     * moment, over the app.
     */
    @SuppressWarnings("deprecation")
    private void hideSystemBars() {
        Window w = getWindow();
        if (Build.VERSION.SDK_INT >= 30) {
            w.setDecorFitsSystemWindows(false);
            WindowInsetsController c = w.getInsetsController();
            if (c != null) {
                c.hide(WindowInsets.Type.statusBars() | WindowInsets.Type.navigationBars());
                c.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            w.addFlags(WindowManager.LayoutParams.FLAG_FULLSCREEN);
            w.getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                    | View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
    }

    @Override
    protected void onDestroy() {
        if (installReceiver != null) {
            try {
                unregisterReceiver(installReceiver);
            } catch (Exception ignored) {
            }
        }
        super.onDestroy();
    }

    /** Whether Android lets this app install packages (Settings ▸ Install unknown apps). */
    public boolean canInstall() {
        return Build.VERSION.SDK_INT < 26 || getPackageManager().canRequestPackageInstalls();
    }

    /** Open the screen where the user allows this app to install packages. */
    public void openInstallSettings() {
        runOnUiThread(new Runnable() { @Override public void run() {
            try {
                Intent i = new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + getPackageName()));
                startActivity(i);
            } catch (Exception ignored) {
            }
        }});
    }

    /** Install the APK at `path` over this app (Android asks the user to confirm). */
    public void installApk(final String path) {
        new Thread(new Runnable() { @Override public void run() {
            try {
                PackageInstaller pi = getPackageManager().getPackageInstaller();
                PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                params.setAppPackageName(getPackageName());
                File f = new File(path);
                params.setSize(f.length());
                int id = pi.createSession(params);
                PackageInstaller.Session session = pi.openSession(id);
                try (InputStream in = new FileInputStream(f); OutputStream out = session.openWrite("base.apk", 0, f.length())) {
                    byte[] buf = new byte[1 << 16];
                    int n;
                    while ((n = in.read(buf)) > 0) {
                        out.write(buf, 0, n);
                    }
                    session.fsync(out);
                }
                Intent done = new Intent(INSTALL_ACTION).setPackage(getPackageName());
                int flags = PendingIntent.FLAG_UPDATE_CURRENT | (Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0);
                PendingIntent pending = PendingIntent.getBroadcast(LightcraftActivity.this, id, done, flags);
                session.commit(pending.getIntentSender());
                session.close();
            } catch (Exception e) {
                nativeInstallStatus(false, String.valueOf(e.getMessage()));
            }
        }}, "lc-install").start();
    }

    /**
     * Show the system picker. `mimes` are the types offered (`image/*` for photos), `multiple`
     * allows several files; each is copied into `destDir` and the copies' paths are reported.
     */
    public void pick(final int id, final String[] mimes, final boolean multiple, final String destDir) {
        runOnUiThread(new Runnable() { @Override public void run() {
            pendingDest = destDir;
            // photos: Android's photo picker (the gallery), where there is one
            if (mimes.length > 0 && mimes[0].startsWith("image/") && hasPhotoPicker()) {
                try {
                    Intent intent = new Intent(MediaStore.ACTION_PICK_IMAGES);
                    intent.setType("image/*");
                    if (multiple) {
                        intent.putExtra(MediaStore.EXTRA_PICK_IMAGES_MAX, MediaStore.getPickImagesMaxLimit());
                    }
                    startActivityForResult(intent, PICK_BASE + id);
                    return;
                } catch (Exception e) {
                    // no photo picker after all: the file picker below
                }
            }
            try {
                Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType(mimes.length == 1 ? mimes[0] : "*/*");
                if (mimes.length > 1) {
                    intent.putExtra(Intent.EXTRA_MIME_TYPES, mimes);
                }
                intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple);
                startActivityForResult(intent, PICK_BASE + id);
            } catch (Exception e) {
                nativePicked(id, new String[0], "The file picker couldn't open: " + e);
            }
        }});
    }

    /** Android 13+, and Android 11-12 with the photo picker update from Google Play. */
    private static boolean hasPhotoPicker() {
        if (Build.VERSION.SDK_INT >= 33) {
            return true;
        }
        return Build.VERSION.SDK_INT >= 30 && SdkExtensions.getExtensionVersion(Build.VERSION_CODES.R) >= 2;
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
