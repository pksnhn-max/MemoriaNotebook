package jp.memorianotebook;

/** JNI bridge to the Rust side (libnotebook_app.so). No logic here:
 *  every call is forwarded to notebook-core via notebook-app. */
public final class Bridge {
    static {
        System.loadLibrary("notebook_app");
    }

    private Bridge() {}

    public static native boolean init(String dataDir, String ortLibPath);
    /** Display name of the live embedder seat (ruri or the BoW fallback). */
    public static native String embedderInfo();
    public static native long createNotebook(String title, long nowSeconds);
    public static native long addTextSource(long notebookId, String title, String text);
    public static native boolean deleteNotebook(long notebookId);
    public static native String ask(long notebookId, String question);
    /** One line per notebook: id \t title \t created_at \t source count. */
    public static native String listNotebooks();
    /** One line per source: id \t title. */
    public static native String listSources(long notebookId);
    /** One line per turn: role \t text \t cited chunk ids (comma-separated). */
    public static native String listTurns(long notebookId);
}
