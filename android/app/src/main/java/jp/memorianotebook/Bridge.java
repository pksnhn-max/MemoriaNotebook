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
    /** Display name of the local generator seat ("未導入" until downloaded). */
    public static native String modelInfo();
    /** Hugging Face GGUF repo search; one repo id per line, or ERR:. */
    public static native String searchModels(String query);
    /** A repo's GGUF files: name \t size per line, or ERR:. */
    public static native String listModelFiles(String repo);
    /** Download one GGUF into the models dir (blocking). "OK bytes" or ERR:. */
    public static native String downloadModel(String repo, String file);
    /** Add a file (epub / txt / md bytes) as a source; source id or -1. */
    public static native long addFileSource(long notebookId, String title, String kind, byte[] bytes);
    /** Web search; title \t url \t snippet per line, or ERR:. */
    public static native String webSearch(String query);
    /** Fetch a page and add it as a source. "OK id" or ERR:. */
    public static native String addWebSource(long notebookId, String title, String url);
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
