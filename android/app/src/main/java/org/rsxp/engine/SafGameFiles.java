package org.rsxp.engine;

import android.app.Activity;
import android.content.ContentResolver;
import android.database.Cursor;
import android.net.Uri;
import android.provider.DocumentsContract;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.security.MessageDigest;
import java.util.Locale;

/** Synchronous SAF operations used by the native Ruby host after a persisted folder grant. */
public final class SafGameFiles {
    private static final String PREFS = "rsxp";
    private static final String TREE_URI = "game_tree_uri";

    private SafGameFiles() {}

    private static Uri tree(Activity activity) {
        String value = activity.getSharedPreferences(PREFS, Activity.MODE_PRIVATE).getString(TREE_URI, null);
        return value == null ? null : Uri.parse(value);
    }

    public static String gameId(Activity activity) {
        Uri tree = tree(activity);
        if (tree == null) return "default";
        try {
            byte[] hash = MessageDigest.getInstance("SHA-256").digest(tree.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8));
            StringBuilder result = new StringBuilder();
            for (int i = 0; i < 12; i++) result.append(String.format(Locale.ROOT, "%02x", hash[i] & 0xff));
            return result.toString();
        } catch (Exception error) {
            return Integer.toHexString(tree.toString().hashCode());
        }
    }

    private static final class Document {
        final String id;
        final String mime;
        Document(String id, String mime) { this.id = id; this.mime = mime; }
        boolean directory() { return DocumentsContract.Document.MIME_TYPE_DIR.equals(mime); }
    }

    private static Document resolve(Activity activity, String relative) throws Exception {
        Uri tree = tree(activity);
        if (tree == null) throw new IllegalStateException("No game folder has been selected");
        String id = DocumentsContract.getTreeDocumentId(tree);
        String mime = DocumentsContract.Document.MIME_TYPE_DIR;
        String normalized = relative == null ? "" : relative.replace('\\', '/');
        if (normalized.startsWith("/") || normalized.matches("^[A-Za-z]:.*")) {
            throw new IllegalArgumentException("Game path must be relative: " + relative);
        }
        ContentResolver resolver = activity.getContentResolver();
        for (String part : normalized.split("/")) {
            if (part.isEmpty() || part.equals(".")) continue;
            if (part.equals("..") || part.indexOf('\0') >= 0) throw new IllegalArgumentException("Invalid game path: " + relative);
            if (!DocumentsContract.Document.MIME_TYPE_DIR.equals(mime)) return null;
            Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id);
            boolean found = false;
            try (Cursor rows = resolver.query(children,
                    new String[]{DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                        DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                        DocumentsContract.Document.COLUMN_MIME_TYPE}, null, null, null)) {
                if (rows == null) throw new IllegalStateException("Cannot list game folder: " + relative);
                while (rows.moveToNext()) {
                    String name = rows.getString(1);
                    if (name != null && name.equalsIgnoreCase(part)) {
                        id = rows.getString(0);
                        mime = rows.getString(2);
                        found = true;
                        break;
                    }
                }
            }
            if (!found) return null;
        }
        return new Document(id, mime);
    }

    public static byte[] readFile(Activity activity, String relative) throws Exception {
        Document document = resolve(activity, relative);
        if (document == null || document.directory()) return null;
        Uri uri = DocumentsContract.buildDocumentUriUsingTree(tree(activity), document.id);
        try (InputStream input = activity.getContentResolver().openInputStream(uri);
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            if (input == null) return null;
            byte[] buffer = new byte[64 * 1024];
            int count;
            while ((count = input.read(buffer)) >= 0) output.write(buffer, 0, count);
            return output.toByteArray();
        }
    }

    public static boolean fileExists(Activity activity, String relative) throws Exception {
        Document document = resolve(activity, relative);
        return document != null && !document.directory();
    }

    public static boolean isDirectory(Activity activity, String relative) throws Exception {
        Document document = resolve(activity, relative);
        return document != null && document.directory();
    }

    /** NUL-separated immediate child names; NUL cannot occur in Android filenames. */
    public static String list(Activity activity, String relative) throws Exception {
        Document document = resolve(activity, relative);
        if (document == null || !document.directory()) return null;
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree(activity), document.id);
        StringBuilder names = new StringBuilder();
        try (Cursor rows = activity.getContentResolver().query(children,
                new String[]{DocumentsContract.Document.COLUMN_DISPLAY_NAME}, null, null, null)) {
            if (rows == null) throw new IllegalStateException("Cannot list game folder: " + relative);
            while (rows.moveToNext()) {
                String name = rows.getString(0);
                if (name == null || name.isEmpty() || name.equals(".") || name.equals("..") || name.indexOf('/') >= 0 || name.indexOf('\\') >= 0) continue;
                if (names.length() > 0) names.append('\0');
                names.append(name);
            }
        }
        return names.toString();
    }
}
