//! Runs Firefox's Bergamot engine (vendor/bergamot/bergamot.wasm, compiled ahead of time by build.rs) in wasmtime.
//! The WASM is built with Emscripten and embind for a browser, so this stands in for its JavaScript glue: it answers
//! the few system calls the engine makes, records what embind registers (classes, constructors, methods, value
//! objects) and calls the engine through those, the way `vendor/bergamot/glue.js` does. Only what translating one
//! line needs is covered; anything else the engine asks for traps, which fails that line and not the app.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use wasmtime::{
    Caller, Config, Engine, ExternType, Func, Instance, Linker, Memory, MemoryType, Module, Result, Store, Table, Val,
    ValType, bail, format_err,
};

use super::model::{self, Pair};

static COMPILED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bergamot.cwasm"));

// 40 MB to start with, as the web's Worker asks for; it grows as the model needs, up to 2 GB
const INITIAL_PAGES: u64 = 640;
const MAX_PAGES: u64 = 32_768;

// The engine's matrix multiplications are imported, so a browser can supply faster ones; these are the WASM's own
const GEMM: &[(&str, &str)] = &[
    ("int8_prepare_a", "int8PrepareAFallback"),
    ("int8_prepare_b", "int8PrepareBFallback"),
    ("int8_prepare_b_from_transposed", "int8PrepareBFromTransposedFallback"),
    ("int8_prepare_b_from_quantized_transposed", "int8PrepareBFromQuantizedTransposedFallback"),
    ("int8_prepare_bias", "int8PrepareBiasFallback"),
    ("int8_multiply_and_add_bias", "int8MultiplyAndAddBiasFallback"),
    ("int8_select_columns_of_b", "int8SelectColumnsOfBFallback"),
];

// Firefox's settings for its int8 "alphas" models (web/src/translate/worker.ts has the same)
const CONFIG: &[(&str, &str)] = &[
    ("beam-size", "1"),
    ("normalize", "1.0"),
    ("word-penalty", "0"),
    ("max-length-break", "128"),
    ("mini-batch-words", "1024"),
    ("workspace", "128"),
    ("max-length-factor", "2.0"),
    ("skip-cost", "true"),
    ("cpu-threads", "0"),
    ("quiet", "true"),
    ("quiet-translation", "true"),
    ("gemm-precision", "int8shiftAlphaAll"),
    ("alignment", "soft"),
];

struct Class {
    name: String,
    destructor: u32,
}

struct ValueObject {
    name: String,
    constructor: u32,
    destructor: u32,
    /// Name, setter, setter context
    fields: Vec<(String, u32, u32)>,
}

/// What embind registered, and the state the engine's system calls need.
struct Host {
    memory: Option<Memory>,
    instance: Option<Instance>,
    classes: HashMap<u32, Class>,
    /// Class, argument count (with the raw constructor), invoker, raw constructor
    constructors: Vec<(u32, u32, u32, u32)>,
    /// (class, method) → invoker, context
    methods: HashMap<(u32, String), (u32, u32)>,
    values: HashMap<u32, ValueObject>,
    /// Classes embind hands out through a std::shared_ptr, with that pointer's destructor
    shared: HashMap<u32, u32>,
    /// Values handed back as `val`: a pointer (a class by value) or a memory view's {size, data}
    emvals: HashMap<i32, [u32; 2]>,
    next_emval: i32,
    start: Instant,
    seed: u64,
}

pub(crate) struct Bergamot {
    store: Store<Host>,
    instance: Instance,
    memory: Memory,
    table: Table,
    service: u32,
    model: u32,
}

impl Bergamot {
    /// Starts the engine with the model in `dir`, which must be whole (`model::is_ready`).
    pub(crate) fn load(dir: &Path, pair: Pair) -> Result<Self> {
        let mut config = Config::new();
        // The engine uses atomics (built with threads), but runs on one thread here
        config.wasm_threads(true);
        let engine = Engine::new(&config)?;
        // SAFETY: COMPILED is what build.rs made with this wasmtime version from bergamot.wasm, never anything from
        // outside the binary; wasmtime still checks it was compiled for this engine and CPU before using it
        let module = unsafe { Module::deserialize(&engine, COMPILED)? };
        let host = Host {
            memory: None,
            instance: None,
            classes: HashMap::new(),
            constructors: Vec::new(),
            methods: HashMap::new(),
            values: HashMap::new(),
            shared: HashMap::new(),
            emvals: HashMap::new(),
            next_emval: 5,
            start: Instant::now(),
            seed: 0x9E37_79B9_7F4A_7C15 ^ u64::from(SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos()),
        };
        let mut store = Store::new(&engine, host);
        let mut linker = Linker::new(&engine);
        let mut memory = None;
        for import in module.imports() {
            let (module_name, name) = (import.module().to_string(), import.name().to_string());
            match import.ty() {
                ExternType::Func(ty) => {
                    let results: Vec<ValType> = ty.results().collect();
                    linker.func_new(&module_name.clone(), &name.clone(), ty, move |mut caller, params, out| {
                        for (slot, t) in out.iter_mut().zip(&results) {
                            *slot = Val::default_for_ty(t).unwrap_or(Val::I32(0));
                        }
                        imported(&mut caller, &module_name, &name, params, out)
                    })?;
                }
                ExternType::Memory(ty) => {
                    let m = Memory::new(
                        &mut store,
                        MemoryType::new(ty.minimum().max(INITIAL_PAGES) as u32, Some(MAX_PAGES as u32)),
                    )?;
                    linker.define(&store, &module_name, &name, m)?;
                    memory = Some(m);
                }
                other => bail!("bergamot imports {module_name}.{name}: {other:?}"),
            }
        }
        let memory = memory.ok_or_else(|| format_err!("bergamot imports no memory"))?;
        let instance = linker.instantiate(&mut store, &module)?;
        store.data_mut().memory = Some(memory);
        store.data_mut().instance = Some(instance);
        instance.get_typed_func::<(), ()>(&mut store, "__wasm_call_ctors")?.call(&mut store, ())?;
        let table = instance
            .get_table(&mut store, "__indirect_function_table")
            .ok_or_else(|| format_err!("bergamot exports no function table"))?;
        let mut b = Self { store, instance, memory, table, service: 0, model: 0 };
        let options = b.value("BlockingServiceConfig", &[("cacheSize", 0)])?;
        b.service = b.construct("BlockingService", &[options])?;
        b.delete_value("BlockingServiceConfig", options)?;
        b.model = b.load_model(dir, pair)?;
        Ok(b)
    }

    fn load_model(&mut self, dir: &Path, pair: Pair) -> Result<u32> {
        let model = self.aligned(&dir.join(model::MODEL), 256)?;
        let lex = self.aligned(&dir.join(model::LEX), 64)?;
        let vocabs = self.construct("AlignedMemoryList", &[])?;
        let names: &[&str] =
            if dir.join(model::VOCAB).exists() { &[model::VOCAB] } else { &[model::SRC_VOCAB, model::TRG_VOCAB] };
        for name in names {
            let vocab = self.aligned(&dir.join(name), 64)?;
            self.call("AlignedMemoryList", "push_back", vocabs, &[Val::I32(vocab as i32)])?;
        }
        let indent = "            ";
        let config: String = CONFIG.iter().map(|(k, v)| format!("{indent}{k}: {v}\n")).collect();
        let config = format!("\n{config}{indent}");
        let (from, to) = pair.langs();
        let strings = [self.string(from)?, self.string(to)?, self.string(&config)?];
        let args = [
            strings[0],
            strings[1],
            strings[2],
            Val::I32(model as i32),
            Val::I32(lex as i32),
            Val::I32(vocabs as i32),
            Val::I32(0),
        ];
        // The model takes the aligned memory over
        let translation_model = self.construct("TranslationModel", &args);
        for s in strings {
            self.free(s)?;
        }
        translation_model
    }

    /// Translates one line.
    pub(crate) fn translate(&mut self, text: &str) -> Result<String> {
        let messages = self.construct("VectorString", &[])?;
        let options = self.construct("VectorResponseOptions", &[])?;
        let result = self.translate_with(messages, options, text);
        self.delete("VectorString", messages)?;
        self.delete("VectorResponseOptions", options)?;
        result
    }

    fn translate_with(&mut self, messages: u32, options: u32, text: &str) -> Result<String> {
        let s = self.string(text)?;
        let pushed = self.call("VectorString", "push_back", messages, &[s]);
        self.free(s)?;
        pushed?;
        let option = self.value("ResponseOptions", &[("qualityScores", 0), ("alignment", 0), ("html", 0)])?;
        let pushed = self.call("VectorResponseOptions", "push_back", options, &[option]);
        self.delete_value("ResponseOptions", option)?;
        pushed?;
        let args = [Val::I32(self.model as i32), Val::I32(messages as i32), Val::I32(options as i32)];
        let responses = self.call("BlockingService", "translate", self.service, &args)?;
        let responses = responses.and_then(|v| v.i32()).ok_or_else(|| format_err!("no responses"))? as u32;
        let first = self.call("VectorResponse", "get", responses, &[Val::I32(0)]);
        let text = first.and_then(|first| {
            let [response, _] = self.take_emval(first)?;
            let text = self.call("Response", "getTranslatedText", response, &[]);
            self.delete("Response", response)?;
            self.take_string(text?)
        });
        self.delete("VectorResponse", responses)?;
        text
    }

    // ---- calling through what embind registered ----

    fn class(&self, name: &str) -> Result<u32> {
        let classes = &self.store.data().classes;
        classes.iter().find(|(_, c)| c.name == name).map(|(k, _)| *k).ok_or_else(|| format_err!("no class {name}"))
    }

    fn dyncall(&mut self, index: u32, args: &[Val]) -> Result<Option<Val>> {
        let func: Func = *self
            .table
            .get(&mut self.store, u64::from(index))
            .and_then(|r| r.as_func().flatten().copied())
            .as_ref()
            .ok_or_else(|| format_err!("no function at {index}"))?;
        let ty = func.ty(&self.store);
        let params: Vec<Val> = ty
            .params()
            .zip(args)
            .map(|(t, a)| match (t, a) {
                // embind passes some numbers as doubles
                (ValType::F64, Val::I32(v)) => Val::F64(f64::from(*v).to_bits()),
                _ => *a,
            })
            .collect();
        let mut out: Vec<Val> = ty.results().map(|t| Val::default_for_ty(&t).unwrap_or(Val::I32(0))).collect();
        func.call(&mut self.store, &params, &mut out)?;
        Ok(out.first().copied())
    }

    fn construct(&mut self, class: &str, args: &[Val]) -> Result<u32> {
        let c = self.class(class)?;
        let &(_, _, invoker, raw) = self
            .store
            .data()
            .constructors
            .iter()
            .find(|(k, argc, _, _)| *k == c && *argc as usize == args.len() + 1)
            .ok_or_else(|| format_err!("no {class} constructor with {} arguments", args.len()))?;
        let mut all = vec![Val::I32(raw as i32)];
        all.extend_from_slice(args);
        Ok(self.dyncall(invoker, &all)?.and_then(|v| v.i32()).unwrap_or(0) as u32)
    }

    fn call(&mut self, class: &str, method: &str, this: u32, args: &[Val]) -> Result<Option<Val>> {
        let c = self.class(class)?;
        let &(invoker, context) = self
            .store
            .data()
            .methods
            .get(&(c, method.to_string()))
            .ok_or_else(|| format_err!("no {class}.{method}"))?;
        // A shared_ptr's method is called on the object inside it
        let this = if self.store.data().shared.contains_key(&c) { read_u32(self.data(), this)? } else { this };
        let mut all = vec![Val::I32(context as i32), Val::I32(this as i32)];
        all.extend_from_slice(args);
        self.dyncall(invoker, &all)
    }

    /// Deletes an object made by a constructor or returned by value, as `.delete()` does in JavaScript
    fn delete(&mut self, class: &str, object: u32) -> Result<()> {
        let c = self.class(class)?;
        let host = self.store.data();
        let destructor = match host.shared.get(&c) {
            Some(&d) => d,
            None => host.classes.get(&c).map(|k| k.destructor).unwrap_or(0),
        };
        self.dyncall(destructor, &[Val::I32(object as i32)])?;
        Ok(())
    }

    /// A value object built the way embind builds one: its constructor, then each field's setter
    fn value(&mut self, name: &str, set: &[(&str, i32)]) -> Result<Val> {
        let (constructor, fields) = {
            let values = &self.store.data().values;
            let v = values.values().find(|v| v.name == name).ok_or_else(|| format_err!("no value {name}"))?;
            (v.constructor, v.fields.clone())
        };
        let ptr = self.dyncall(constructor, &[])?.and_then(|v| v.i32()).ok_or_else(|| format_err!("no {name}"))?;
        for (field, setter, context) in fields {
            let x = set.iter().find(|(n, _)| *n == field).map(|(_, x)| *x);
            let x = x.ok_or_else(|| format_err!("{name}.{field} not given"))?;
            self.dyncall(setter, &[Val::I32(context as i32), Val::I32(ptr), Val::I32(x)])?;
        }
        Ok(Val::I32(ptr))
    }

    fn delete_value(&mut self, name: &str, value: Val) -> Result<()> {
        let destructor = self.store.data().values.values().find(|v| v.name == name).map(|v| v.destructor);
        self.dyncall(destructor.ok_or_else(|| format_err!("no value {name}"))?, &[value])?;
        Ok(())
    }

    fn take_emval(&mut self, v: Option<Val>) -> Result<[u32; 2]> {
        let handle = v.and_then(|v| v.i32()).ok_or_else(|| format_err!("no val"))?;
        self.store.data_mut().emvals.remove(&handle).ok_or_else(|| format_err!("unknown val {handle}"))
    }

    // ---- memory ----

    fn data(&self) -> &[u8] {
        self.memory.data(&self.store)
    }

    fn malloc(&mut self, n: u32) -> Result<u32> {
        let f = self.instance.get_typed_func::<u32, u32>(&mut self.store, "malloc")?;
        match f.call(&mut self.store, n)? {
            0 => bail!("out of memory"),
            p => Ok(p),
        }
    }

    fn free(&mut self, ptr: Val) -> Result<()> {
        let f = self.instance.get_typed_func::<u32, ()>(&mut self.store, "free")?;
        f.call(&mut self.store, ptr.i32().unwrap_or(0) as u32)
    }

    /// A std::string as embind passes it: a malloc'd {length, bytes}, freed by the caller after the call
    fn string(&mut self, s: &str) -> Result<Val> {
        let len = u32::try_from(s.len())?;
        let p = self.malloc(len + 5)?;
        let data = self.memory.data_mut(&mut self.store);
        write_u32(data, p, len)?;
        slice_mut(data, p + 4, len + 1)?.copy_from_slice(&[s.as_bytes(), &[0]].concat());
        Ok(Val::I32(p as i32))
    }

    /// A std::string returned to JavaScript: a malloc'd {length, bytes} that is ours to free
    fn take_string(&mut self, v: Option<Val>) -> Result<String> {
        let p = v.ok_or_else(|| format_err!("no string"))?;
        let at = p.i32().unwrap_or(0) as u32;
        let data = self.data();
        let len = read_u32(data, at)?;
        let s = String::from_utf8_lossy(slice(data, at + 4, len)?).into_owned();
        self.free(p)?;
        Ok(s)
    }

    /// A file in the engine's AlignedMemory, read straight into the WASM's memory
    fn aligned(&mut self, path: &Path, alignment: i32) -> Result<u32> {
        let mut file = File::open(path)?;
        let len = u32::try_from(file.metadata()?.len())?;
        let m = self.construct("AlignedMemory", &[Val::I32(len as i32), Val::I32(alignment)])?;
        let view = self.call("AlignedMemory", "getByteArrayView", m, &[])?;
        let [size, at] = self.take_emval(view)?;
        if size != len {
            bail!("aligned memory is {size} bytes, not {len}");
        }
        file.read_exact(slice_mut(self.memory.data_mut(&mut self.store), at, len)?)?;
        Ok(m)
    }
}

fn slice(data: &[u8], at: u32, len: u32) -> Result<&[u8]> {
    data.get(at as usize..at as usize + len as usize).ok_or_else(|| format_err!("{at}+{len} is outside memory"))
}

fn slice_mut(data: &mut [u8], at: u32, len: u32) -> Result<&mut [u8]> {
    data.get_mut(at as usize..at as usize + len as usize).ok_or_else(|| format_err!("{at}+{len} is outside memory"))
}

fn read_u32(data: &[u8], at: u32) -> Result<u32> {
    let bytes: [u8; 4] = slice(data, at, 4)?.try_into()?;
    Ok(u32::from_le_bytes(bytes))
}

fn write_u32(data: &mut [u8], at: u32, v: u32) -> Result<()> {
    slice_mut(data, at, 4)?.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn cstr(data: &[u8], at: u32) -> Result<String> {
    let rest = data.get(at as usize..).ok_or_else(|| format_err!("{at} is outside memory"))?;
    let end = rest.iter().position(|&b| b == 0).unwrap_or(0);
    Ok(String::from_utf8_lossy(rest.get(..end).unwrap_or_default()).into_owned())
}

fn arg(params: &[Val], n: usize) -> u32 {
    params.get(n).and_then(|v| v.i32()).unwrap_or(0) as u32
}

/// Byte ranges of sentences, standing in for the Intl.Segmenter the glue gives the engine
fn sentences(text: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut cut = false;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        if cut && !c.is_whitespace() {
            out.push((start as u32, at as u32));
            start = at;
            cut = false;
        }
        let ends = matches!(c, '。' | '！' | '？' | '…')
            || (matches!(c, '.' | '!' | '?') && next.is_none_or(|n| n.is_whitespace()));
        if ends && !matches!(next, Some('。' | '！' | '？' | '.' | '!' | '?' | '”' | '"' | '）' | ')')) {
            cut = true;
        }
    }
    if start < text.len() {
        out.push((start as u32, text.len() as u32));
    }
    out
}

fn host_malloc(caller: &mut Caller<'_, Host>, n: u32) -> Result<u32> {
    let instance = caller.data().instance.ok_or_else(|| format_err!("not started"))?;
    let f = instance.get_typed_func::<u32, u32>(&mut *caller, "malloc")?;
    f.call(&mut *caller, n)
}

/// What the engine imports: embind's registrations, the matrix multiplications and a few system calls
fn imported(caller: &mut Caller<'_, Host>, module: &str, name: &str, p: &[Val], r: &mut [Val]) -> Result<()> {
    if module == "wasm_gemm" {
        let target = GEMM.iter().find(|(n, _)| *n == name).ok_or_else(|| format_err!("no gemm {name}"))?.1;
        let instance = caller.data().instance.ok_or_else(|| format_err!("not started"))?;
        let f = instance.get_func(&mut *caller, target).ok_or_else(|| format_err!("no {target}"))?;
        return f.call(&mut *caller, p, r);
    }
    let memory = caller.data().memory.ok_or_else(|| format_err!("no memory"))?;
    let set = |r: &mut [Val], v: Val| {
        if let Some(slot) = r.first_mut() {
            *slot = v;
        }
    };
    match name {
        "_embind_register_class" => {
            let class = Class { name: cstr(memory.data(&caller), arg(p, 10))?, destructor: arg(p, 12) };
            caller.data_mut().classes.insert(arg(p, 0), class);
        }
        "_embind_register_class_constructor" => {
            caller.data_mut().constructors.push((arg(p, 0), arg(p, 1), arg(p, 4), arg(p, 5)));
        }
        "_embind_register_class_function" => {
            let method = cstr(memory.data(&caller), arg(p, 1))?;
            caller.data_mut().methods.insert((arg(p, 0), method), (arg(p, 5), arg(p, 6)));
        }
        "_embind_register_value_object" => {
            let value = ValueObject {
                name: cstr(memory.data(&caller), arg(p, 1))?,
                constructor: arg(p, 3),
                destructor: arg(p, 5),
                fields: Vec::new(),
            };
            caller.data_mut().values.insert(arg(p, 0), value);
        }
        "_embind_register_value_object_field" => {
            let field = cstr(memory.data(&caller), arg(p, 1))?;
            if let Some(v) = caller.data_mut().values.get_mut(&arg(p, 0)) {
                v.fields.push((field, arg(p, 8), arg(p, 9)));
            }
        }
        "_embind_register_smart_ptr" => {
            caller.data_mut().shared.insert(arg(p, 1), arg(p, 11));
        }
        n if n.starts_with("_embind_") || n == "_emval_incref" || n == "_emval_decref" => {}
        "_emval_take_value" => {
            let data = memory.data(&caller);
            let words = [read_u32(data, arg(p, 1))?, read_u32(data, arg(p, 1) + 4).unwrap_or(0)];
            let host = caller.data_mut();
            let handle = host.next_emval;
            host.next_emval = host.next_emval.checked_add(1).unwrap_or(5).max(5);
            host.emvals.insert(handle, words);
            set(r, Val::I32(handle));
        }
        // The glue's one piece of inline JavaScript: splitting text into sentences
        "emscripten_asm_const_int" => {
            let data = memory.data(&caller);
            let args = arg(p, 2);
            let word = |k: u32| read_u32(data, args + 4 * k);
            let (text, count, starts_out, ends_out) = (word(0)?, word(2)?, word(3)?, word(4)?);
            let parts = sentences(&cstr(data, text)?);
            let n = u32::try_from(parts.len())?;
            let starts = host_malloc(caller, 4 * n.max(1))?;
            let ends = host_malloc(caller, 4 * n.max(1))?;
            let data = memory.data_mut(&mut *caller);
            for (k, (s, e)) in (0u32..).zip(&parts) {
                write_u32(data, starts + 4 * k, *s)?;
                write_u32(data, ends + 4 * k, *e)?;
            }
            write_u32(data, count, n)?;
            write_u32(data, starts_out, starts)?;
            write_u32(data, ends_out, ends)?;
        }
        "emscripten_get_now" => set(r, Val::F64((caller.data().start.elapsed().as_secs_f64() * 1000.0).to_bits())),
        "_emscripten_date_now" => {
            let ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as f64;
            set(r, Val::F64(ms.to_bits()));
        }
        "_emscripten_get_now_is_monotonic" => set(r, Val::I32(1)),
        // 2 GB, as an unsigned number
        "emscripten_get_heap_max" => set(r, Val::I32(i32::MIN)),
        "emscripten_resize_heap" => {
            let want = u64::from(arg(p, 0));
            let have = memory.data_size(&caller) as u64;
            let pages = want.saturating_sub(have).div_ceil(65_536);
            set(r, Val::I32(i32::from(memory.grow(&mut *caller, pages).is_ok())));
        }
        "emscripten_memcpy_big" => {
            let (to, from, n) = (arg(p, 0) as usize, arg(p, 1) as usize, arg(p, 2) as usize);
            let data = memory.data_mut(&mut *caller);
            if from.saturating_add(n) > data.len() || to.saturating_add(n) > data.len() {
                bail!("memcpy outside memory");
            }
            data.copy_within(from..from + n, to);
        }
        // Only seeds the engine's random numbers, which beam size 1 doesn't use for anything that matters
        "getentropy" => {
            let mut x = caller.data().seed;
            for b in slice_mut(memory.data_mut(&mut *caller), arg(p, 0), arg(p, 1))? {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *b = x as u8;
            }
            caller.data_mut().seed = x;
            set(r, Val::I32(0));
        }
        "__cxa_allocate_exception" => set(r, Val::I32(host_malloc(caller, arg(p, 0) + 16)? as i32 + 16)),
        // The engine's log, quiet with this config: swallowed, but counted as written
        "fd_write" => {
            let data = memory.data(&caller);
            let (iovs, count, out) = (arg(p, 1), arg(p, 2), arg(p, 3));
            let mut total = 0u32;
            for k in 0..count {
                total = total.saturating_add(read_u32(data, iovs + 8 * k + 4)?);
            }
            write_u32(memory.data_mut(&mut *caller), out, total)?;
        }
        "environ_sizes_get" => {
            let data = memory.data_mut(&mut *caller);
            write_u32(data, arg(p, 0), 0)?;
            write_u32(data, arg(p, 1), 0)?;
        }
        "environ_get" | "_tzset_js" | "_localtime_js" | "strftime_l" | "setTempRet0" => {}
        // EBADF: there are no files
        "fd_close" | "fd_seek" | "fd_read" => set(r, Val::I32(8)),
        // ENOSYS
        n if n.starts_with("__syscall_") || n == "_mmap_js" || n == "_munmap_js" => set(r, Val::I32(-52)),
        "__assert_fail" => {
            let data = memory.data(&caller);
            bail!("bergamot assert {} at {}:{}", cstr(data, arg(p, 0))?, cstr(data, arg(p, 1))?, arg(p, 2));
        }
        // abort, exit, __cxa_throw: the line fails
        other => bail!("bergamot called {module}.{other}"),
    }
    Ok(())
}
