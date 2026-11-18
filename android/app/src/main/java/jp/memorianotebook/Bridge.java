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
    /** One cited chunk: source title, tab, chunk text (escaped), or empty. */
    public static native String getChunk(long chunkId);
    /** Persist API settings and return short status. */
    public static native String setApiConfig(String seat, String baseUrl, String apiKey, String model);
    /** Return current API settings as JSON. */
    public static native String getApiConfig();
    /** Fetch available models from API endpoint; JSON array of ids or {"error":"..."}. */
    public static native String fetchApiModels(String baseUrl, String apiKey);
    /** Voice catalog as JSON: key/name/credit/termsUrl/sizeMb/vvmFile/installed/consented/isA/isB. */
    public static native String getVoiceCatalog();
    /** Record consent to one voice's terms. "OK" or ERR:. */
    public static native String recordVoiceConsent(String key);
    /** Download one voice's VVM from the official repo (blocking). "OK bytes" or ERR:. */
    public static native String downloadVoice(String key);
    /** Set the dialogue pair (host A / host B voice keys). "OK" or ERR:. */
    public static native String setVoicePair(String a, String b);
    /** Generate an audio overview (blocking): script via API, synthesis on-device.
     *  Returns the summary record as JSON, or ERR:. */
    public static native String generateVoiceSummary(long notebookId, String focus, String dictDir, String ortLib, long atMillis);
    /** Audio summaries of one notebook as a JSON array, newest first. */
    public static native String listVoiceSummaries(long notebookId);
    /** Delete one audio summary and its WAV file. "OK" or ERR:. */
    public static native String deleteVoiceSummary(long id);
}
