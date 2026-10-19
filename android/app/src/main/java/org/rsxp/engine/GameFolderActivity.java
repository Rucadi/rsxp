package org.rsxp.engine;

import android.app.Activity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.Typeface;
import android.net.Uri;
import android.os.Bundle;
import android.os.Build;
import android.view.Gravity;
import android.view.View;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;

/** Imports a user-selected RPG Maker project into private storage for native CRuby. */
public final class GameFolderActivity extends Activity {
    private static final int PICK_GAME = 17;
    private static final int BG = Color.rgb(13, 19, 31);
    private static final int PANEL = Color.rgb(25, 35, 52);
    private static final int TEXT = Color.rgb(239, 244, 252);
    private static final int MUTED = Color.rgb(166, 181, 202);
    private static final int ACCENT = Color.rgb(103, 163, 255);
    private LinearLayout content;
    private Button chooseButton;
    private Button changeButton;
    private TextView status;
    private ProgressBar progress;
    private boolean rubyReady;
    private boolean importing;
    private Uri selectedTree;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().setStatusBarColor(BG);
        getWindow().setNavigationBarColor(BG);
        getWindow().getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR & 0);
        buildHome();
        new Thread(() -> {
            try {
                installRubyRuntime();
                runOnUiThread(() -> {
                    rubyReady = true;
                    String saved = getSharedPreferences("rsxp", MODE_PRIVATE).getString("game_tree_uri", null);
                    selectedTree = saved == null ? null : Uri.parse(saved);
                    status.setText(selectedTree == null
                            ? "Native Ruby is ready. Choose a game folder to grant direct access and launch."
                            : "Native Ruby is ready. Your selected game stays in its original folder.");
                    chooseButton.setEnabled(true);
                    chooseButton.setText(selectedTree == null ? "Choose game folder" : "Launch selected game");
                    chooseButton.setOnClickListener(v -> {
                        if (selectedTree == null) openPicker(); else launchGame();
                    });
                    changeButton.setVisibility(selectedTree == null ? View.GONE : View.VISIBLE);
                });
            } catch (Exception error) {
                runOnUiThread(() -> showError("Could not prepare the native Ruby runtime.", error.toString()));
            }
        }, "rsxp-ruby-install").start();
    }

    @Override protected void onResume() {
        super.onResume();
        File report = new File(getFilesDir(), "last_error.txt");
        if (!report.isFile()) return;
        try {
            byte[] bytes;
            try (java.io.FileInputStream in = new java.io.FileInputStream(report)) {
                bytes = new byte[(int) report.length()];
                int offset = 0;
                while (offset < bytes.length) {
                    int count = in.read(bytes, offset, bytes.length - offset);
                    if (count < 0) break;
                    offset += count;
                }
                if (offset != bytes.length) bytes = java.util.Arrays.copyOf(bytes, offset);
            }
            report.delete();
            showError("The game stopped while starting or running.", new String(bytes, java.nio.charset.StandardCharsets.UTF_8));
        } catch (Exception error) {
            showError("The game stopped, and its error report could not be read.", error.toString());
        }
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request != PICK_GAME) return;
        if (result != RESULT_OK || data == null || data.getData() == null) {
            importing = false;
            status.setText("Folder selection canceled. Your selected game remains available.");
            chooseButton.setEnabled(rubyReady);
            return;
        }
        Uri tree = data.getData();
        try {
            int grantFlags = data.getFlags() & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
            getContentResolver().takePersistableUriPermission(tree, grantFlags & Intent.FLAG_GRANT_READ_URI_PERMISSION);
        } catch (Exception error) {
            showError("Android could not grant access to that folder.", error.toString());
            return;
        }
        getSharedPreferences("rsxp", MODE_PRIVATE).edit().putString("game_tree_uri", tree.toString()).apply();
        selectedTree = tree;
        showBusy("Checking game folder", "Checking for Game.ini. Game files will stay in their original folder.");
        new Thread(() -> {
            try {
                if (!SafGameFiles.fileExists(this, "Game.ini")) {
                    runOnUiThread(() -> showError("This folder does not look like an RPG Maker game.", "Choose the project folder containing Game.ini at its top level."));
                    return;
                }
                runOnUiThread(() -> {
                    buildHome();
                    status.setText("Game folder selected. Launching without copying its files…");
                    chooseButton.setText("Launch selected game");
                    chooseButton.setEnabled(rubyReady);
                    chooseButton.setOnClickListener(v -> launchGame());
                    changeButton.setVisibility(View.VISIBLE);
                    launchGame();
                });
            } catch (Exception error) {
                runOnUiThread(() -> showError("Android could not read the selected game folder.", error.toString()));
            }
        }, "rsxp-game-check").start();
    }

    private void buildHome() {
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        content = new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        content.setGravity(Gravity.CENTER_VERTICAL);
        content.setPadding(dp(28), dp(28), dp(28), dp(28));
        content.setBackgroundColor(BG);
        scroll.addView(content);

        TextView brand = text("RSXP", 34, TEXT, true);
        content.addView(brand, match());
        TextView subtitle = text("RPG Maker runtime for Android", 16, MUTED, false);
        LinearLayout.LayoutParams subParams = match();
        subParams.topMargin = dp(4);
        content.addView(subtitle, subParams);

        LinearLayout card = new LinearLayout(this);
        card.setOrientation(LinearLayout.VERTICAL);
        card.setPadding(dp(22), dp(22), dp(22), dp(22));
        card.setBackgroundColor(PANEL);
        LinearLayout.LayoutParams cardParams = match();
        cardParams.topMargin = dp(28);
        content.addView(card, cardParams);

        TextView heading = text("Load a game", 22, TEXT, true);
        card.addView(heading, match());
        status = text("Preparing the native Ruby runtime…", 15, MUTED, false);
        status.setLineSpacing(dp(3), 1f);
        LinearLayout.LayoutParams statusParams = match();
        statusParams.topMargin = dp(10);
        card.addView(status, statusParams);
        progress = new ProgressBar(this);
        progress.setIndeterminate(true);
        LinearLayout.LayoutParams progressParams = new LinearLayout.LayoutParams(dp(32), dp(32));
        progressParams.topMargin = dp(18);
        card.addView(progress, progressParams);
        chooseButton = button("Preparing…", true);
        chooseButton.setEnabled(false);
        LinearLayout.LayoutParams buttonParams = match();
        buttonParams.topMargin = dp(20);
        card.addView(chooseButton, buttonParams);
        chooseButton.setOnClickListener(v -> openPicker());
        changeButton = button("Choose another game folder", false);
        changeButton.setVisibility(View.GONE);
        LinearLayout.LayoutParams changeParams = match();
        changeParams.topMargin = dp(8);
        card.addView(changeButton, changeParams);
        changeButton.setOnClickListener(v -> openPicker());

        TextView hint = text("Select the folder containing Game.ini. RSXP reads game files from that folder and keeps game saves in app storage.", 13, MUTED, false);
        hint.setLineSpacing(dp(3), 1f);
        LinearLayout.LayoutParams hintParams = match();
        hintParams.topMargin = dp(18);
        content.addView(hint, hintParams);
        setContentView(scroll);
    }

    private void openPicker() {
        if (!rubyReady || importing) return;
        Intent pick = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        pick.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        startActivityForResult(pick, PICK_GAME);
    }

    private void launchGame() {
        if (selectedTree == null) {
            openPicker();
            return;
        }
        startActivity(new Intent(this, android.app.NativeActivity.class));
    }

    private void showBusy(String title, String message) {
        content.removeAllViews();
        TextView brand = text("RSXP", 34, TEXT, true);
        content.addView(brand, match());
        TextView heading = text(title, 22, TEXT, true);
        LinearLayout.LayoutParams hp = match(); hp.topMargin = dp(26); content.addView(heading, hp);
        status = text(message, 15, MUTED, false);
        LinearLayout.LayoutParams sp = match(); sp.topMargin = dp(10); content.addView(status, sp);
        progress = new ProgressBar(this);
        progress.setIndeterminate(true);
        LinearLayout.LayoutParams pp = new LinearLayout.LayoutParams(dp(40), dp(40)); pp.topMargin = dp(24); content.addView(progress, pp);
        content.setGravity(Gravity.CENTER_VERTICAL);
    }

    private void showError(String summary, String details) {
        if (content == null) buildHome();
        content.removeAllViews();
        TextView brand = text("RSXP", 34, TEXT, true);
        content.addView(brand, match());
        TextView heading = text("Something went wrong", 23, TEXT, true);
        LinearLayout.LayoutParams hp = match(); hp.topMargin = dp(24); content.addView(heading, hp);
        TextView message = text(summary, 15, MUTED, false);
        LinearLayout.LayoutParams mp = match(); mp.topMargin = dp(8); content.addView(message, mp);
        TextView detailView = text(details == null || details.isEmpty() ? "No additional details were provided." : details, 12, Color.rgb(220, 227, 238), false);
        detailView.setTextIsSelectable(true);
        detailView.setPadding(dp(14), dp(12), dp(14), dp(12));
        detailView.setBackgroundColor(PANEL);
        detailView.setMaxHeight(dp(180));
        LinearLayout.LayoutParams dpv = match(); dpv.topMargin = dp(16); content.addView(detailView, dpv);
        Button copy = button("Copy error details", false);
        LinearLayout.LayoutParams cp = match(); cp.topMargin = dp(12); content.addView(copy, cp);
        copy.setOnClickListener(v -> {
            ClipboardManager clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
            clipboard.setPrimaryClip(ClipData.newPlainText("RSXP error", summary + "\n\n" + details));
            Toast.makeText(this, "Error details copied", Toast.LENGTH_SHORT).show();
        });
        Button retry = button("Choose another game folder", true);
        LinearLayout.LayoutParams rp = match(); rp.topMargin = dp(10); content.addView(retry, rp);
        retry.setEnabled(rubyReady);
        retry.setOnClickListener(v -> openPicker());
        setContentView(content.getParent() instanceof ScrollView ? (View) content.getParent() : content);
    }

    private TextView text(String value, int size, int color, boolean bold) {
        TextView view = new TextView(this);
        view.setText(value); view.setTextSize(size); view.setTextColor(color);
        if (bold) view.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        return view;
    }

    private Button button(String label, boolean primary) {
        Button b = new Button(this);
        b.setText(label); b.setAllCaps(false); b.setTextSize(15); b.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        b.setTextColor(primary ? Color.rgb(12, 24, 42) : TEXT);
        b.setBackgroundTintList(ColorStateList.valueOf(primary ? ACCENT : Color.rgb(52, 67, 88)));
        return b;
    }

    private LinearLayout.LayoutParams match() { return new LinearLayout.LayoutParams(-1, -2); }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }

    private void remove(File file) {
        if (file.isDirectory()) { File[] children = file.listFiles(); if (children != null) for (File child : children) remove(child); }
        file.delete();
    }

    private void installRubyRuntime() throws Exception {
        String abi = Build.SUPPORTED_ABIS[0];
        File destination = new File(getFilesDir(), "ruby/" + abi);
        File marker = new File(destination, ".cruby-3.4.7-monitor-v2");
        if (marker.isFile()) return;
        remove(destination);
        copyAssets("ruby/" + abi, destination);
        if (!marker.createNewFile()) throw new Exception("Cannot mark Ruby runtime installation");
    }

    private void copyAssets(String assetPath, File target) throws Exception {
        String[] children = getAssets().list(assetPath);
        if (children == null || children.length == 0) {
            File parent = target.getParentFile();
            if (parent != null) parent.mkdirs();
            try (InputStream in = getAssets().open(assetPath); FileOutputStream out = new FileOutputStream(target)) {
                byte[] buffer = new byte[64 * 1024];
                int count;
                while ((count = in.read(buffer)) >= 0) out.write(buffer, 0, count);
            }
            return;
        }
        if (!target.mkdirs() && !target.isDirectory()) throw new Exception("Cannot create " + target);
        for (String child : children) copyAssets(assetPath + "/" + child, new File(target, child));
    }
}
