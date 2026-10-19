// Interface simple entre Lumen (Rust) et whisper.cpp, pour iPhone et iPad : iOS
// interdit le programme à part `lumen-whisper`, Whisper tourne donc dans l'app.
// Il y partage la ggml de llama.cpp (une seule copie dans le programme, sans
// conflit de symboles) : whisper.cpp 1.9.4 est compilé avec ses en-têtes.
// Les structures de whisper.h restent ici, du côté C : Rust n'en voit que des
// fonctions aux types simples (src/whisper.rs).
#include "whisper.h"

#include <cstring>

extern "C" {

struct lw_token {
    int32_t id;
    int64_t t0;
    int64_t t1;
    int64_t t_dtw;
    const char * text;
};

static whisper_alignment_heads_preset preset_of(const char * name) {
    if (!name || !*name) return WHISPER_AHEADS_NONE;
    struct { const char * n; whisper_alignment_heads_preset p; } all[] = {
        {"tiny.en", WHISPER_AHEADS_TINY_EN},   {"tiny", WHISPER_AHEADS_TINY},           {"base.en", WHISPER_AHEADS_BASE_EN},
        {"base", WHISPER_AHEADS_BASE},         {"small.en", WHISPER_AHEADS_SMALL_EN},   {"small", WHISPER_AHEADS_SMALL},
        {"medium.en", WHISPER_AHEADS_MEDIUM_EN}, {"medium", WHISPER_AHEADS_MEDIUM},     {"large-v1", WHISPER_AHEADS_LARGE_V1},
        {"large-v2", WHISPER_AHEADS_LARGE_V2}, {"large-v3", WHISPER_AHEADS_LARGE_V3},   {"large-v3-turbo", WHISPER_AHEADS_LARGE_V3_TURBO},
    };
    for (auto & a : all)
        if (strcmp(a.n, name) == 0) return a.p;
    return WHISPER_AHEADS_NONE;
}

// `preset` : têtes d'attention pour l'alignement DTW (« large-v3-turbo »…), vide : sans DTW.
void * lw_init(const char * model, int use_gpu, const char * preset) {
    whisper_context_params cp = whisper_context_default_params();
    cp.use_gpu = use_gpu != 0;
    whisper_alignment_heads_preset p = preset_of(preset);
    if (p != WHISPER_AHEADS_NONE) {
        // l'alignement DTW exige l'attention classique
        cp.flash_attn = false;
        cp.dtw_token_timestamps = true;
        cp.dtw_aheads_preset = p;
    }
    return whisper_init_from_file_with_params(model, cp);
}

typedef void (*lw_progress)(int, void *);
typedef bool (*lw_abort)(void *);

struct lw_callbacks {
    lw_progress progress;
    void * progress_user;
};

static void on_progress(struct whisper_context *, struct whisper_state *, int p, void * user) {
    auto * cb = (lw_callbacks *) user;
    if (cb->progress) cb->progress(p, cb->progress_user);
}

// Transcrit `n` échantillons (mono, 16 kHz). `beam` : 0 pour le décodage glouton.
int lw_full(void * ctx, const float * pcm, int n, const char * lang, int threads, int beam, lw_progress progress, void * progress_user,
            lw_abort abort, void * abort_user) {
    whisper_full_params p = whisper_full_default_params(beam > 0 ? WHISPER_SAMPLING_BEAM_SEARCH : WHISPER_SAMPLING_GREEDY);
    if (beam > 0) p.beam_search.beam_size = beam;
    p.n_threads = threads;
    p.language = lang;
    p.translate = false;
    p.print_special = false;
    p.print_progress = false;
    p.print_realtime = false;
    p.print_timestamps = false;
    p.token_timestamps = true;
    p.suppress_blank = true;
    lw_callbacks cb = {progress, progress_user};
    p.progress_callback = on_progress;
    p.progress_callback_user_data = &cb;
    p.abort_callback = abort;
    p.abort_callback_user_data = abort_user;
    return whisper_full((whisper_context *) ctx, p, pcm, n);
}

int lw_n_segments(void * ctx) { return whisper_full_n_segments((whisper_context *) ctx); }

int lw_n_tokens(void * ctx, int seg) { return whisper_full_n_tokens((whisper_context *) ctx, seg); }

int lw_eot(void * ctx) { return whisper_token_eot((whisper_context *) ctx); }

void lw_token(void * ctx, int seg, int i, lw_token * out) {
    auto * c = (whisper_context *) ctx;
    whisper_token_data d = whisper_full_get_token_data(c, seg, i);
    out->id = d.id;
    out->t0 = d.t0;
    out->t1 = d.t1;
    out->t_dtw = d.t_dtw;
    out->text = whisper_token_to_str(c, d.id);
}

void lw_free(void * ctx) {
    if (ctx) whisper_free((whisper_context *) ctx);
}

}
