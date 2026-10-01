// SPDX-License-Identifier: Apache-2.0
//! The English words code is written in, for the Spanish words people ask with.
//!
//! # The problem
//! Search is over words. A question asked in Spanish about code written in English shares almost
//! none of them: "¿cómo se estima el número de tokens?" and `estimate_tokens` have one word in
//! common, and the question's other words match whatever happens to contain "el" or "se". Most
//! code is written in English whatever language its authors speak, so for a Spanish speaker this
//! is the common case, not the rare one.
//!
//! # The answer, and why it is this small
//! A fixed glossary of the words programming and business software are built from: verbs such as
//! *validar*, *guardar*, *calcular*, and nouns such as *usuario*, *pedido*, *factura*. A question
//! keeps its own words, so code documented in Spanish is found as before, and gains the English
//! ones beside them. Nothing is guessed and nothing leaves the machine: a word is in the glossary
//! or it is not.
//!
//! # Matching
//! Accents are folded first, so *número* and *numero* are the same word. An entry is either a whole
//! word or a stem marked with `*`, and a stem matches only when what follows it is a Spanish
//! ending ([`ENDINGS`]): *valid\** matches *valida*, *validar* and *validación*, and *pag\** would
//! have matched the English *page*, which is why that entry is written out word by word instead.
//! When several stems match, the longest wins, so *pagina* is a page and not a payment.
//!
//! # What it does not do
//! It knows about 180 words and stems, not the language. A word it does not know is searched as written,
//! which is exactly what happened to every word before it existed.

use crate::memory::tongue::folded;

/// Spanish words and stems, with the English words that name the same thing in code.
///
/// A trailing `*` marks a stem. Every key is lowercase and has no accents.
const ES_EN: &[(&str, &[&str])] = &[
    // ---- verbs: what code does
    ("abr*", &["open"]),
    ("actualiz*", &["update"]),
    ("agreg*", &["add"]),
    ("anad*", &["add"]),
    ("analiz*", &["parse", "analyze"]),
    ("anul*", &["cancel", "void"]),
    ("aplic*", &["apply"]),
    ("asign*", &["assign"]),
    ("autentic*", &["auth", "authenticate"]),
    ("autoriz*", &["authorize", "permission"]),
    ("borr*", &["delete", "remove"]),
    ("busc*", &["search", "find"]),
    ("calcul*", &["calculate", "compute"]),
    ("cambi*", &["change"]),
    ("carg*", &["load"]),
    ("cerr*", &["close"]),
    ("cobr*", &["charge", "payment"]),
    ("compar*", &["compare"]),
    ("comprob*", &["check", "verify"]),
    ("conect*", &["connect"]),
    ("configur*", &["config", "configuration"]),
    ("consult*", &["query"]),
    ("convert*", &["convert"]),
    ("copi*", &["copy"]),
    ("crea", &["create"]),
    ("crean", &["create"]),
    ("crear", &["create"]),
    ("creado", &["create"]),
    ("creada", &["create"]),
    ("creacion", &["create"]),
    ("descarg*", &["download"]),
    ("deten*", &["stop"]),
    ("devolv*", &["return", "refund"]),
    ("devuelv*", &["return", "refund"]),
    ("dibuj*", &["draw", "render"]),
    ("divid*", &["split", "divide"]),
    ("edit*", &["edit"]),
    ("elimin*", &["delete", "remove"]),
    ("empiez*", &["start"]),
    ("envi*", &["send"]),
    ("escrib*", &["write"]),
    ("estim*", &["estimate"]),
    ("export*", &["export"]),
    ("filtr*", &["filter"]),
    ("firm*", &["sign", "signature"]),
    ("gener*", &["generate"]),
    ("guard*", &["save", "store"]),
    ("import*", &["import"]),
    ("imprim*", &["print"]),
    ("indexa*", &["index"]),
    ("inici*", &["start", "init"]),
    ("instal*", &["install"]),
    ("lee", &["read"]),
    ("leen", &["read"]),
    ("leer", &["read"]),
    ("leido", &["read"]),
    ("lectura", &["read"]),
    ("limpi*", &["clear", "clean"]),
    ("llam*", &["call"]),
    ("mostr*", &["show", "display", "render"]),
    ("muestr*", &["show", "display", "render"]),
    ("mover", &["move"]),
    ("mueve", &["move"]),
    ("notific*", &["notify", "notification"]),
    ("obten*", &["get", "fetch"]),
    ("obtien*", &["get", "fetch"]),
    ("orden*", &["order", "sort"]),
    ("paga", &["pay", "payment"]),
    ("pagar", &["pay", "payment"]),
    ("pagado", &["pay", "payment"]),
    ("pagada", &["pay", "payment"]),
    ("pago", &["pay", "payment"]),
    ("pagos", &["pay", "payment"]),
    ("pars*", &["parse"]),
    ("proces*", &["process"]),
    ("recib*", &["receive", "receipt"]),
    ("recuper*", &["recover", "fetch", "retrieve"]),
    ("redim*", &["redeem"]),
    ("registr*", &["register", "record", "log"]),
    ("renderiz*", &["render"]),
    ("reserv*", &["reserve", "booking"]),
    ("resolv*", &["resolve"]),
    ("resuelv*", &["resolve"]),
    ("respond*", &["respond", "response"]),
    ("restaur*", &["restore"]),
    ("revis*", &["review", "check"]),
    ("sincroniz*", &["sync"]),
    ("suscri*", &["subscribe", "subscription"]),
    ("transform*", &["transform"]),
    ("valid*", &["validate"]),
    ("vend*", &["sell", "sale"]),
    ("verific*", &["verify", "check"]),
    // ---- nouns: what code is about
    ("almacen*", &["store", "storage", "warehouse"]),
    ("archiv*", &["file"]),
    ("arista*", &["edge"]),
    ("busqued*", &["search"]),
    ("cantidad*", &["quantity", "amount"]),
    ("carrito*", &["cart"]),
    ("categori*", &["category"]),
    ("client*", &["client", "customer"]),
    ("clave*", &["key", "password"]),
    ("codigo*", &["code"]),
    ("comando*", &["command"]),
    ("comentari*", &["comment"]),
    ("compania*", &["company"]),
    ("conexion*", &["connection"]),
    ("contrasena*", &["password"]),
    ("correo*", &["mail", "email"]),
    ("costo*", &["cost"]),
    ("cuenta*", &["account", "bill"]),
    ("cupon*", &["coupon"]),
    ("dato*", &["data"]),
    ("descuento*", &["discount"]),
    ("direccion*", &["address"]),
    ("documento*", &["document", "doc"]),
    ("empleado*", &["employee"]),
    ("empresa*", &["company"]),
    ("entrada*", &["input", "entry"]),
    ("etiqueta*", &["label", "tag"]),
    ("evento*", &["event"]),
    ("factur*", &["invoice", "bill"]),
    ("fecha*", &["date"]),
    ("fichero*", &["file"]),
    ("funcion*", &["function"]),
    ("grafo*", &["graph"]),
    ("hora", &["time", "hour"]),
    ("horas", &["time", "hour"]),
    ("impuesto*", &["tax"]),
    ("indice*", &["index"]),
    ("informe*", &["report"]),
    ("inventari*", &["inventory", "stock"]),
    ("lenguaje*", &["language"]),
    ("limite*", &["limit"]),
    ("linea*", &["line"]),
    ("llave*", &["key"]),
    ("memori*", &["memory"]),
    ("mensaje*", &["message"]),
    ("mesa", &["table"]),
    ("mesas", &["table"]),
    ("metodo*", &["method"]),
    ("modulo*", &["module"]),
    ("moneda*", &["currency"]),
    ("monto*", &["amount"]),
    ("nodo*", &["node"]),
    ("nombre*", &["name"]),
    ("numero*", &["number", "count"]),
    ("objeto*", &["object"]),
    ("pagina*", &["page"]),
    ("palabra*", &["word"]),
    ("pantalla*", &["screen"]),
    ("pedido*", &["order"]),
    ("permiso*", &["permission"]),
    ("plantilla*", &["template"]),
    ("precio*", &["price"]),
    ("presupuesto*", &["budget"]),
    ("producto*", &["product"]),
    ("propina*", &["tip"]),
    ("proveedor*", &["supplier", "vendor", "provider"]),
    ("prueba*", &["test"]),
    ("pregunta*", &["question", "query"]),
    ("recibo*", &["receipt"]),
    ("regla*", &["rule"]),
    ("reporte*", &["report"]),
    ("respuesta*", &["response", "answer"]),
    ("rol", &["role"]),
    ("roles", &["role"]),
    ("ruta*", &["route", "path"]),
    ("salida*", &["output", "exit"]),
    ("servidor*", &["server"]),
    ("sesion*", &["session"]),
    ("simbolo*", &["symbol"]),
    ("sucursal*", &["branch"]),
    ("tabla*", &["table"]),
    ("tarea*", &["task"]),
    ("tarjeta*", &["card"]),
    ("tiempo*", &["time"]),
    ("tienda*", &["shop", "store"]),
    ("tipo*", &["type"]),
    ("transacci*", &["transaction"]),
    ("usuari*", &["user"]),
    ("valor*", &["value"]),
    ("venta*", &["sale"]),
    ("vista*", &["view"]),
];

/// What may follow a stem for it to match: the endings of Spanish verbs and nouns.
///
/// `s` makes a plural of a stem that already ends in its vowel (*pedido\** gives *pedidos*). `e` is
/// here because so many nouns end in it (*cliente*, *nombre*), which is also why every stem that an
/// English word could continue with an `e` is written out word by word instead.
const ENDINGS: [&str; 28] = [
    "", "s", "a", "as", "an", "ar", "e", "es", "en", "er", "ir", "o", "os", "ado", "ada", "ados",
    "adas", "ido", "ida", "idos", "idas", "ando", "iendo", "acion", "aciones", "cion", "ciones",
    "amos",
];

/// The English words for one Spanish word, or none when the glossary does not know it.
///
/// # Examples
/// ```text
/// english_for("Número")  == ["number", "count"]
/// english_for("validar") == ["validate"]
/// english_for("page")    == []
/// ```
pub(super) fn english_for(word: &str) -> &'static [&'static str] {
    let word = folded(word);
    let mut best: Option<(usize, &'static [&'static str])> = None;
    for &(key, english) in ES_EN {
        let matches = match key.strip_suffix('*') {
            Some(stem) => word
                .strip_prefix(stem)
                .is_some_and(|ending| ENDINGS.contains(&ending)),
            None => word == key,
        };
        let length = key.len();
        if matches && best.is_none_or(|(longest, _)| length > longest) {
            best = Some((length, english));
        }
    }
    best.map_or(&[], |(_, english)| english)
}

/// The English words for a list of Spanish words, each once, in the order they were asked, leaving
/// out any that is already among the words.
pub(super) fn translate(words: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in words {
        for english in english_for(word) {
            let english = (*english).to_owned();
            if !words.contains(&english) && !out.contains(&english) {
                out.push(english);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{ENDINGS, ES_EN, english_for, translate};
    use crate::memory::tongue::folded;

    /// Accents and case do not matter.
    #[test]
    fn accents_are_folded() {
        assert_eq!(folded("Número"), "numero");
        assert_eq!(folded("CONTRASEÑA"), "contrasena");
        assert_eq!(english_for("número"), english_for("NUMERO"));
    }

    /// A stem matches its inflections and nothing an English word could continue with.
    #[test]
    fn stems_match_spanish_endings_only() {
        for word in [
            "valida",
            "validar",
            "validación",
            "validado",
            "validaciones",
        ] {
            assert_eq!(english_for(word), ["validate"], "{word}");
        }
        assert_eq!(english_for("cupones"), ["coupon"]);
        assert_eq!(english_for("estima"), ["estimate"]);
        assert!(english_for("page").is_empty());
        assert!(english_for("validated").is_empty());
        assert!(english_for("zanahoria").is_empty());
    }

    /// The longest matching stem wins: a page is not a payment.
    #[test]
    fn the_longest_stem_wins() {
        assert_eq!(english_for("pagina"), ["page"]);
        assert_eq!(english_for("pagos"), ["pay", "payment"]);
        assert_eq!(english_for("notificaciones"), ["notify", "notification"]);
    }

    /// A question gains each English word once, and none it already had.
    #[test]
    fn translation_adds_each_word_once() {
        let words: Vec<String> = ["estima", "numero", "tokens", "number"]
            .iter()
            .map(|w| (*w).to_owned())
            .collect();
        assert_eq!(translate(&words), ["estimate", "count"]);
    }

    /// Every key is lowercase, unaccented and unique, and every ending is distinct.
    #[test]
    fn the_glossary_is_well_formed() {
        let mut keys: Vec<&str> = ES_EN.iter().map(|(key, _)| *key).collect();
        for key in &keys {
            assert_eq!(folded(key), *key, "{key}");
            assert!(key.trim_end_matches('*').len() >= 3, "{key}");
        }
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "a key is listed twice");
        let mut endings = ENDINGS.to_vec();
        endings.sort_unstable();
        endings.dedup();
        assert_eq!(endings.len(), ENDINGS.len());
    }
}
