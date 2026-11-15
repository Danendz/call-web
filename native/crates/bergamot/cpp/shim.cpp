// A C interface to bergamot-translator's BlockingService, set up as the WASM build is in Firefox and in
// `engine/src/translate/host.rs`: one model, one line at a time, no quality scores, alignment or HTML. Sentences are
// split by ssplit, as in every native build; the WASM build's languages only pick its browser's sentence splitter.

#include <cstdlib>
#include <cstring>
#include <exception>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "translator/byte_array_util.h"
#include "translator/service.h"

using marian::bergamot::AlignedMemory;
using marian::bergamot::BlockingService;
using marian::bergamot::MemoryBundle;
using marian::bergamot::ResponseOptions;
using marian::bergamot::TranslationModel;

struct BergamotTranslator {
  BergamotTranslator(const BlockingService::Config& config, std::shared_ptr<TranslationModel> model)
      : service(config), model(std::move(model)) {}
  BlockingService service;
  std::shared_ptr<TranslationModel> model;
};

namespace {

// A copy for Rust, which frees it with bergamot_free_string
char* copy(const std::string& s) {
  char* out = static_cast<char*>(std::malloc(s.size() + 1));
  if (out != nullptr) {
    std::memcpy(out, s.data(), s.size());
    out[s.size()] = '\0';
  }
  return out;
}

void fail(char** error, const char* what) {
  if (error != nullptr) *error = copy(what);
}

BlockingService::Config serviceConfig() {
  BlockingService::Config config;
  config.cacheSize = 0;
  return config;
}

}  // namespace

extern "C" {

// Loads a model from its files. `trg_vocab` may be null when one vocabulary serves both languages. On failure returns
// null and sets `*error`.
BergamotTranslator* bergamot_new(const char* config, const char* model, const char* lex, const char* src_vocab,
                                 const char* trg_vocab, char** error) {
  try {
    MemoryBundle memory;
    memory.models.emplace_back(marian::bergamot::loadFileToMemory(model, 256));
    memory.shortlist = marian::bergamot::loadFileToMemory(lex, 64);
    auto source = std::make_shared<AlignedMemory>(marian::bergamot::loadFileToMemory(src_vocab, 64));
    memory.vocabs.push_back(source);
    if (trg_vocab != nullptr) {
      memory.vocabs.push_back(std::make_shared<AlignedMemory>(marian::bergamot::loadFileToMemory(trg_vocab, 64)));
    } else {
      memory.vocabs.push_back(source);
    }
    auto model = std::make_shared<TranslationModel>(std::string(config), std::move(memory));
    return new BergamotTranslator{serviceConfig(), std::move(model)};
  } catch (const std::exception& e) {
    fail(error, e.what());
  } catch (...) {
    fail(error, "unknown error");
  }
  return nullptr;
}

// Translates `len` bytes of UTF-8. On failure returns null and sets `*error`.
char* bergamot_translate(BergamotTranslator* translator, const char* text, size_t len, char** error) {
  try {
    std::vector<std::string> source{std::string(text, len)};
    std::vector<ResponseOptions> options(1);
    auto responses = translator->service.translateMultiple(translator->model, std::move(source), options);
    if (responses.empty()) {
      fail(error, "no response");
      return nullptr;
    }
    return copy(responses.front().target.text);
  } catch (const std::exception& e) {
    fail(error, e.what());
  } catch (...) {
    fail(error, "unknown error");
  }
  return nullptr;
}

void bergamot_free_string(char* s) { std::free(s); }

void bergamot_delete(BergamotTranslator* translator) { delete translator; }

}  // extern "C"
