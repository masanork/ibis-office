package dev.sorane.docs;

import android.app.Activity;
import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.OpenableColumns;
import android.system.Os;
import android.text.InputType;
import android.util.Log;
import android.view.KeyEvent;
import android.view.View;
import android.view.Window;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import android.widget.FrameLayout;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.Locale;
import java.util.concurrent.ConcurrentHashMap;

public class IbisActivity extends NativeActivity {
    private static final String TAG = "ibisOffice";
    private static final int REQ_OPEN_DOCUMENT = 1001;
    private static final int REQ_CREATE_DOCUMENT = 1002;

    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private ImeBridgeView imeView;
    private long pendingOpenRequestId = 0;
    private long pendingSaveRequestId = 0;
    private volatile boolean nativeReady = false;
    private Uri pendingOpenUri = null;
    private final ConcurrentHashMap<String, Uri> exportUriByTempPath = new ConcurrentHashMap<>();

    public static native void nativeOnFilePicked(long requestId, String path);
    public static native void nativeOnSavePathPicked(long requestId, String path);
    public static native void nativeOnOpenPath(String path);
    public static native void nativeOnImeCommit(String text);
    public static native void nativeOnImeComposing(String text);
    public static native void nativeOnImeBackspace();
    public static native void nativeOnImeEnter();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        try {
            File filesDir = getFilesDir();
            File dataDir = new File(filesDir, "data");
            File configDir = new File(filesDir, "config");
            dataDir.mkdirs();
            configDir.mkdirs();
            Os.setenv("HOME", filesDir.getAbsolutePath(), true);
            Os.setenv("XDG_DATA_HOME", dataDir.getAbsolutePath(), true);
            Os.setenv("XDG_CONFIG_HOME", configDir.getAbsolutePath(), true);
            String langTag = Locale.getDefault().toLanguageTag().replace('-', '_');
            Os.setenv("LANG", langTag + ".UTF-8", true);
        } catch (Throwable t) {
            Log.w(TAG, "Failed to set env", t);
        }

        super.onCreate(savedInstanceState);

        Window window = getWindow();
        if (window != null) {
            window.setStatusBarColor(Color.WHITE);
            window.setNavigationBarColor(Color.WHITE);
            View decor = window.getDecorView();
            if (decor != null) {
                int flags = decor.getSystemUiVisibility();
                flags |= View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR;
                flags |= View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
                decor.setSystemUiVisibility(flags);
            }
        }
        View content = findViewById(android.R.id.content);
        if (content instanceof FrameLayout) {
            FrameLayout frame = (FrameLayout) content;
            frame.setOnApplyWindowInsetsListener((v, insets) -> {
                int top = insets.getSystemWindowInsetTop();
                int bottom = insets.getSystemWindowInsetBottom();
                if (frame.getChildCount() > 0) {
                    View nativeView = frame.getChildAt(0);
                    if (nativeView.getLayoutParams() instanceof FrameLayout.LayoutParams) {
                        FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) nativeView.getLayoutParams();
                        if (lp.topMargin != top || lp.bottomMargin != bottom) {
                            lp.topMargin = top;
                            lp.bottomMargin = bottom;
                            nativeView.setLayoutParams(lp);
                        }
                    }
                }
                return insets.consumeSystemWindowInsets();
            });
        }

        imeView = new ImeBridgeView(this);
        addContentView(imeView, new FrameLayout.LayoutParams(1, 1));
        handleOpenIntent(getIntent());
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleOpenIntent(intent);
    }

    public void onNativeReady() {
        final Uri uri;
        synchronized (this) {
            nativeReady = true;
            uri = pendingOpenUri;
            pendingOpenUri = null;
        }
        if (uri != null) {
            dispatchOpenUri(uri);
        }
    }

    private void handleOpenIntent(Intent intent) {
        if (intent == null) return;
        final Uri uri;
        if (Intent.ACTION_VIEW.equals(intent.getAction())) {
            uri = intent.getData();
        } else if (Intent.ACTION_SEND.equals(intent.getAction())
                && intent.getType() != null) {
            Object stream = intent.getParcelableExtra(Intent.EXTRA_STREAM);
            uri = stream instanceof Uri ? (Uri) stream : null;
        } else {
            return;
        }
        if (uri == null) return;
        synchronized (this) {
            if (!nativeReady) {
                pendingOpenUri = uri;
                return;
            }
        }
        dispatchOpenUri(uri);
    }

    private void dispatchOpenUri(final Uri uri) {
        new Thread(() -> {
            String path = copyUriToStorage(uri);
            if (path != null) {
                nativeOnOpenPath(path);
            }
        }).start();
    }

    public void showKeyboard() {
        mainHandler.post(() -> {
            if (imeView == null) return;
            imeView.requestFocus();
            InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm != null) {
                imm.showSoftInput(imeView, InputMethodManager.SHOW_IMPLICIT);
            }
        });
    }

    public void hideKeyboard() {
        mainHandler.post(() -> {
            if (imeView == null) return;
            InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm != null) {
                imm.hideSoftInputFromWindow(imeView.getWindowToken(), 0);
            }
        });
    }

    public void openUrl(String url) {
        mainHandler.post(() -> {
            try {
                Intent intent = new Intent(Intent.ACTION_VIEW, Uri.parse(url));
                startActivity(intent);
            } catch (Throwable t) {
                Log.w(TAG, "Failed to open URL: " + url, t);
            }
        });
    }

    public String getClipboardText() {
        final String[] result = new String[1];
        final Object lock = new Object();
        Runnable task = () -> {
            synchronized (lock) {
                try {
                    ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                    if (cm != null && cm.hasPrimaryClip()) {
                        ClipData clip = cm.getPrimaryClip();
                        if (clip != null && clip.getItemCount() > 0) {
                            CharSequence cs = clip.getItemAt(0).coerceToText(this);
                            if (cs != null) {
                                result[0] = cs.toString();
                            }
                        }
                    }
                } catch (Throwable t) {
                    Log.w(TAG, "Clipboard read failed", t);
                }
                lock.notifyAll();
            }
        };
        if (Looper.myLooper() == Looper.getMainLooper()) {
            task.run();
        } else {
            synchronized (lock) {
                mainHandler.post(task);
                try {
                    lock.wait(1000);
                } catch (InterruptedException ignored) {}
            }
        }
        return result[0];
    }

    public void setClipboardText(String text) {
        mainHandler.post(() -> {
            try {
                ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                if (cm != null) {
                    cm.setPrimaryClip(ClipData.newPlainText("ibisOffice", text));
                }
            } catch (Throwable t) {
                Log.w(TAG, "Clipboard write failed", t);
            }
        });
    }

    public void pickFileForOpen(long requestId) {
        mainHandler.post(() -> {
            pendingOpenRequestId = requestId;
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType("*/*");
            startActivityForResult(intent, REQ_OPEN_DOCUMENT);
        });
    }

    public void pickPathForSave(long requestId, String suggestedName) {
        mainHandler.post(() -> {
            pendingSaveRequestId = requestId;
            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType("*/*");
            if (suggestedName != null && !suggestedName.isEmpty()) {
                intent.putExtra(Intent.EXTRA_TITLE, suggestedName);
            }
            startActivityForResult(intent, REQ_CREATE_DOCUMENT);
        });
    }

    public boolean syncSavedFile(String localPath) {
        Uri uri = exportUriByTempPath.get(localPath);
        if (uri == null) {
            return false;
        }
        try (InputStream in = new FileInputStream(localPath);
             OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
            if (out == null) return false;
            byte[] buf = new byte[16384];
            int n;
            while ((n = in.read(buf)) >= 0) {
                out.write(buf, 0, n);
            }
            out.flush();
            return true;
        } catch (Throwable t) {
            Log.e(TAG, "Failed to sync saved file to SAF URI", t);
            return false;
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQ_OPEN_DOCUMENT) {
            final long reqId = pendingOpenRequestId;
            pendingOpenRequestId = 0;
            final Uri pickedUri = (resultCode == Activity.RESULT_OK && data != null) ? data.getData() : null;
            if (pickedUri == null) {
                nativeOnFilePicked(reqId, null);
            } else {
                new Thread(() -> nativeOnFilePicked(reqId, copyUriToStorage(pickedUri))).start();
            }
        } else if (requestCode == REQ_CREATE_DOCUMENT) {
            long reqId = pendingSaveRequestId;
            pendingSaveRequestId = 0;
            String tempPath = null;
            if (resultCode == Activity.RESULT_OK && data != null && data.getData() != null) {
                Uri uri = data.getData();
                String name = queryDisplayName(uri);
                if (name == null || name.isEmpty()) name = "document.docx";
                File dir = new File(getCacheDir(), "saf_save");
                dir.mkdirs();
                File outFile = new File(dir, System.currentTimeMillis() + "_" + name);
                tempPath = outFile.getAbsolutePath();
                exportUriByTempPath.put(tempPath, uri);
            }
            nativeOnSavePathPicked(reqId, tempPath);
        }
    }

    private String copyUriToStorage(Uri uri) {
        String name = queryDisplayName(uri);
        if (name == null || name.isEmpty()) name = "document";
        File dir = new File(getFilesDir(), "opened");
        dir.mkdirs();
        File outFile = new File(dir, System.currentTimeMillis() + "_" + name);
        try (InputStream in = getContentResolver().openInputStream(uri);
             OutputStream out = new FileOutputStream(outFile)) {
            if (in == null) return null;
            byte[] buf = new byte[16384];
            int n;
            while ((n = in.read(buf)) >= 0) {
                out.write(buf, 0, n);
            }
            out.flush();
            String path = outFile.getAbsolutePath();
            exportUriByTempPath.put(path, uri);
            return path;
        } catch (Throwable t) {
            Log.e(TAG, "Failed to copy picked URI to cache", t);
            return null;
        }
    }

    private String queryDisplayName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(uri, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int idx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (idx >= 0) return cursor.getString(idx);
            }
        } catch (Throwable ignored) {}
        String last = uri.getLastPathSegment();
        return last != null ? new File(last).getName() : null;
    }

    private static final class ImeBridgeView extends View {
        ImeBridgeView(Context context) {
            super(context);
            setFocusable(true);
            setFocusableInTouchMode(true);
        }

        @Override
        public boolean onCheckIsTextEditor() {
            return true;
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
            outAttrs.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE;
            outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_ACTION_NONE;
            return new BaseInputConnection(this, false) {
                @Override
                public boolean commitText(CharSequence text, int newCursorPosition) {
                    nativeOnImeCommit(text != null ? text.toString() : "");
                    return true;
                }

                @Override
                public boolean setComposingText(CharSequence text, int newCursorPosition) {
                    nativeOnImeComposing(text != null ? text.toString() : "");
                    return true;
                }

                @Override
                public boolean finishComposingText() {
                    nativeOnImeComposing("");
                    return true;
                }

                @Override
                public boolean deleteSurroundingText(int beforeLength, int afterLength) {
                    for (int i = 0; i < beforeLength; i++) {
                        nativeOnImeBackspace();
                    }
                    return true;
                }

                @Override
                public boolean sendKeyEvent(KeyEvent event) {
                    if (event.getAction() == KeyEvent.ACTION_DOWN) {
                        int code = event.getKeyCode();
                        if (code == KeyEvent.KEYCODE_DEL) {
                            nativeOnImeBackspace();
                            return true;
                        } else if (code == KeyEvent.KEYCODE_ENTER) {
                            nativeOnImeEnter();
                            return true;
                        }
                    }
                    return super.sendKeyEvent(event);
                }
            };
        }
    }
}
