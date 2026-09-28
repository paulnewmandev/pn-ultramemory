<p align="center">
  <img src="assets/logo/banner.svg" alt="pn-ultramemory" width="380">
</p>

<h1 align="center">pn-ultramemory</h1>

<p align="center">
  <b>Una memoria local, consciente del código, para agentes de programación.</b><br>
  Menos tokens, mejor recuperación, un solo binario.
</p>

<p align="center">
  <a href="README.md">🇬🇧 English</a> ·
  <a href="README.es.md">🇪🇸 Español</a>
</p>

<p align="center">
  <a href="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/guards.yml"><img alt="Guards" src="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/guards.yml/badge.svg?branch=main"></a>
  <a href="LICENSE"><img alt="Licencia: Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-blue"></a>
  <a href="https://www.rust-lang.org"><img alt="Hecho con Rust" src="https://img.shields.io/badge/built%20with-Rust-000000?logo=rust&logoColor=white"></a>
  <img alt="Rust edición 2024" src="https://img.shields.io/badge/edition-2024-orange?logo=rust&logoColor=white">
  <img alt="código unsafe prohibido" src="https://img.shields.io/badge/unsafe-forbidden-success">
  <img alt="1335 pruebas pasando" src="https://img.shields.io/badge/tests-1335%20passing-success">
</p>

<p align="center">
  <img alt="Telemetría: ninguna" src="https://img.shields.io/badge/telemetry-none-success">
  <img alt="Red: nunca" src="https://img.shields.io/badge/network-never-success">
  <img alt="Precio: gratis" src="https://img.shields.io/badge/price-free%20forever-blueviolet">
  <img alt="Estado: proyecto nuevo" src="https://img.shields.io/badge/estado-proyecto%20nuevo-orange">
  <a href="docs/releasing.md"><img alt="SemVer 2.0.0" src="https://img.shields.io/badge/semver-2.0.0-3f4551"></a>
</p>

---

## El problema

Tu agente de programación lee ficheros enteros para responder preguntas sobre tu código. Casi nada
de lo que lee es la respuesta, y pagas cada token.

**pn-ultramemory** indexa tu repositorio en un grafo de símbolos y las relaciones entre ellos, y
responde una pregunta con una **cápsula**: el código que importa, al nivel de detalle que cabe,
dentro del presupuesto de tokens que tú fijas.

También recuerda las decisiones y lecciones que le cuentas, **ancladas al código que describen**,
así que cuando ese código cambia la memoria lo dice en vez de volverse mentira en silencio.

Un binario estático. Nunca abre una conexión de red. No cuesta nada.

<p align="center">
  <img src="assets/diagrams/why.svg" alt="Leer los ficheros cuesta 16.109 tokens y acierta el 85% de las veces; una cápsula cuesta 413 y acierta el 99%" width="760">
</p>

---

## Lo que ahorra

Cada cifra sale de `pn-ultramemory bench` sobre este repositorio. Nada está proyectado.

| Estilo de pregunta | Presupuesto | Encuentra el código | Tokens | Leer ficheros | **Ahorro** |
|---|---:|---:|---:|---:|---:|
| Por descripción | 500 | **99 %** | 413 | 16.109 | **97,4 %** |
| Por descripción | 1000 | **99 %** | 802 | 16.109 | **95,0 %** |
| Por descripción | 2000 | **99 %** | 1.270 | 16.109 | **92,1 %** |
| Por nombre | 500 | **98 %** | 424 | 16.258 | **97,4 %** |
| Por nombre | 1000 | **100 %** | 718 | 16.258 | **95,6 %** |

> **De dónde salen estas cifras, y dónde no valen.** Están medidas sobre el código de esta misma
> herramienta: un repositorio Rust de unos 260 ficheros. El ahorro depende del tamaño del proyecto,
> porque lo que se ahorra es lo que habría costado leer ficheros enteros. En un sitio pequeño o un
> puñado de scripts, leerlos nunca fue caro, así que hay menos que ahorrar. Mide el tuyo con
> `pn-ultramemory bench`.

<sub>100 tareas por fila, Apple M5. La base de comparación lee los tres ficheros que mejor puntúa una
búsqueda por palabras, que es lo que hace un agente sin índice — y solo acierta el 85 % desde una
descripción y el 41 % desde un nombre, costando cuarenta veces más. `task_success` se informa como
`unobservable`: si tu agente resolvió o no el problema queda fuera de lo que esta herramienta puede
observar, y lo dice en vez de inventarse un número.</sub>

### Velocidad

| Operación | 266 ficheros · 4.880 símbolos · 22.458 aristas |
|---|---|
| Índice completo desde cero | **343 ms** |
| Reindexar sin cambios | **13 ms** |
| Un `recall` | **7,2 ms** mediana · 8,7 ms al percentil 95 |

### Se mantiene plano al crecer

El coste no es aquí una afirmación, es una **prueba**. La suite barre el tamaño del repositorio y
afirma que `recall`, `map` y `outline` cuestan lo mismo a cualquier tamaño, cada una emparejada con
una medida de control que **debe crecer** sobre el mismo barrido — así una prueba que dejó de medir
falla en vez de pasar en silencio.

---

## En la terminal

```console
$ pn-ultramemory index
indexing ━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 266/266
✓ 266 files, 4880 symbols, 22458 edges  in 0.34s
```

Una barra de progreso real: el indexador informa de cada fichero al terminarlo, así que el número
que se mueve es trabajo hecho y no una estimación.

Los resultados van en color — claves en negrita, cifras en azul, un tic verde al acertar. Todo eso
aparece **solo cuando lee una persona**. Si canalizas la salida recibes los mismos bytes de siempre:

```console
$ pn-ultramemory recall "cómo se estiman los tokens" -b 300 | wc -c   # cero secuencias de escape
$ pn-ultramemory -f json stats | jq .index.symbols                    # JSON válido
```

`NO_COLOR` y `TERM=dumb` apagan el color en todas partes. La barra va a la salida de error, así que
nunca toca el resultado que estás capturando. Ejecutarlo en una terminal también imprime la marca:

```
●───◉───●  ╔═╗╔╗╔   ╦ ╦╦  ╔╦╗╦═╗╔═╗╔╦╗╔═╗╔╦╗╔═╗╦═╗╦ ╦
 ╲  │  ╱   ╠═╝║║║═══║ ║║   ║ ╠╦╝╠═╣║║║║╣ ║║║║ ║╠╦╝╚╦╝
  ╰─●─╯    ╩  ╝╚╝   ╚═╝╩═╝ ╩ ╩╚═╩ ╩╩ ╩╚═╝╩ ╩╚═╝╩╚═ ╩
```

---

## Instalación

> Si le vas a pedir a un agente que lo instale por ti, dale el enlace de este repositorio y dile que
> siga [AGENTS.md](AGENTS.md). Está escrito para que lo lea un modelo y lo ejecute paso a paso.

```bash
git clone https://github.com/paulnewmandev/pn-ultramemory
cd pn-ultramemory
cargo build --release          # Rust edición 2024
```

```bash
pn-ultramemory index           # construye el grafo
pn-ultramemory install --agents claude-code   # o cursor, codex, gemini, windsurf, zed, …
                                              # sin --agents se registra en todos los que
                                              # encuentre; --dry-run enseña qué tocaría
                                              # sin cambiar nada
pn-ultramemory doctor          # comprueba que todo funcionó
```

---

## Las órdenes

### Recuperar

```console
$ pn-ultramemory recall "how are tokens estimated" -b 600
capsule:
  query: how are tokens estimated
  budget: 600
  used: 525
  omitted: 15
files[9]{f,path}:
  3,crates/codec/src/tokens.rs
symbols[5]{id,f,lines,kind,name,d,text}:
  7890647377742991523,3,149-157,method,"Features::estimate",L3,"fn estimate(self) -> f64 - Combines the counts into an estimated number of tokens."
also[12]: MAX_CONTEXT_TOKENS,measure,symbol_cost,printed_tokens,estimate_tokens,…
```

| Orden | Qué responde |
|---|---|
| `recall <pregunta>` | *¿Qué código importa para esto?* — empaquetado en un presupuesto |
| `outline <ruta>` | *¿Qué hay en este fichero?* — todos los símbolos, por una fracción de los tokens |
| `expand <símbolo>` | *Enséñame el código* — en ventanas de líneas |
| `impact <símbolo>` | *¿Qué rompo si cambio esto?* — y cómo de seguro es |
| `map` | *¿Qué es este repositorio?* — entero, dentro de un presupuesto |
| `graph` | *Dibújalo* — Mermaid, DOT, SVG o JSON |

### Recordar

| Orden | Qué hace |
|---|---|
| `remember <tipo> "<texto>" --about <símbolo>` | Guarda una decisión anclada al código |
| `memories --stale` | Lo que puede haber dejado de ser cierto |
| `reanchor <id>` · `forget <id>` | Confírmala, o descártala |
| `feedback <señal> --memory <id>` | Dile cómo salió un resultado |
| `learn status` · `learn why` · `learn reset` | Inspecciona o borra lo aprendido |

Nueve tipos: `decision`, `fact`, `lesson`, `dead-end`, `error-fix`, `convention`, `requirement`,
`task`, `session`.

### Informar y conectar

| Orden | Qué hace |
|---|---|
| `report --as html\|pdf\|md --lang en\|es` | Un informe que puedes enviar |
| `docs gaps` · `docs apply` · `docs build` | Símbolos sin documentar, aplicar docs, generar referencia |
| `stats` · `bench` | Números, y el benchmark de arriba |
| `serve` · `mcp-config` | MCP por entrada y salida estándar, y el fragmento que conecta un cliente |
| `install` · `uninstall` · `doctor` | Configuración, reversible de forma exacta |
| `toon encode\|decode` · `completions` | Conversión de formato, autocompletado |

---

## Flags

### En todas partes

| Flag | Por defecto | Qué hace |
|---|---|---|
| `-C, --repo <RUTA>` | el `.git` padre más cercano | El repositorio sobre el que trabajar |
| `--data-dir <RUTA>` | tu directorio de datos | Dónde vive el índice — nunca dentro de tu repositorio |
| `-f, --format <toon\|json\|text>` | `toon` | `toon` es compacto, `json` para programas, `text` para personas |
| `--delimiter <comma\|tab\|pipe>` | `comma` | Separador de columnas; el tabulador cuesta algo menos |
| `-q, --quiet` | apagado | Sin progreso ni avisos — solo el resultado |
| `--no-metrics` | apagado | No registra ningún contador de uso |
| `-h, --help` · `-V, --version` | | |

Entorno: `PN_ULTRAMEMORY_REPO`, `PN_ULTRAMEMORY_HOME`, `PN_ULTRAMEMORY_NO_METRICS`,
`PN_ULTRAMEMORY_NO_HOOKS`, `PN_ULTRAMEMORY_SESSION`, `NO_COLOR`.

### Por orden

| Orden | Flags |
|---|---|
| `index` | `--force` reanaliza todo · `--threads <N>` |
| `recall` | `-b, --budget <TOKENS>` · `--explain` por qué está cada símbolo · `--path <PREFIJO>` |
| `outline` | `-b, --budget <TOKENS>` |
| `expand` | `--from <LÍNEA>` · `--to <LÍNEA>` |
| `impact` | `--depth <N>` hasta 5 · `--min-confidence <guess\|heuristic\|resolved\|exact>` · `--limit <N>` |
| `graph` | `--modules` · `--module-depth <N>` · `--depth <N>` · `--max-nodes <N>` · `--min-confidence` · `--as <mermaid\|dot\|svg\|json>` · `--lang <en\|es>` · `-o, --out <FICHERO>` |
| `map` | `-b, --budget <TOKENS>` · `--path <PREFIJO>` |
| `remember` | `--about <SÍMBOLO>` repetible · `--by <user\|agent\|tool>` |
| `memories` | `--kind <TIPO>` · `--stale` · `--limit <N>` |
| `feedback` | `--memory <ID>` · señal: `useful`, `used`, `ignored`, `dead-end`, `corrected` |
| `report` | `--lang <en\|es>` · `--as <html\|pdf\|md>` · `-o, --out <FICHERO>` · `--module-depth <N>` · `--title <TEXTO>` |
| `bench` | `--tasks <N>` · `--seed <N>` · `-b, --budget <TOKENS>` repetible · `--baseline-files <N>` |
| `docs gaps` | `--path <PREFIJO>` · `--limit <N>` · `--context` |
| `docs apply` | `<FICHERO>` o `-` para entrada estándar · `--dry-run` |
| `docs build` | `--path <PREFIJO>` · `--title <TEXTO>` · `-o, --out <FICHERO>` |
| `install` | `--agents <ID>` repetible · `--scope <user\|project>` · `--dry-run` · `--force` · `--name <NOMBRE>` · `--command <RUTA>` · `--pin-repo` |

---

## Cómo funciona

<p align="center">
  <img src="assets/diagrams/how.svg" alt="Tu código se indexa en un grafo; una pregunta busca semillas, recorre el grafo, ordena y empaqueta los candidatos dentro de un presupuesto, y la cápsula se mide antes de devolverla" width="760">
</p>

### Cinco resoluciones por símbolo

<p align="center">
  <img src="assets/diagrams/levels.svg" alt="Cada símbolo puede mostrarse en uno de cinco niveles, desde su nombre solo hasta su código entero, cada uno más caro que el anterior" width="760">
</p>

`recall` resuelve una **mochila multi-resolución**: un nivel por símbolo, maximizando la utilidad
dentro del presupuesto. Comprobado contra un solucionador exacto por programación dinámica sobre
3.000 instancias aleatorias.

Cuando el presupuesto aprieta **degrada por escalones** en vez de romperse: fuente completa, luego
firmas, luego una lista de nombres. Con 150 tokens sigue devolviendo cinco nombres útiles, que es lo
que lo hace funcionar para un modelo pequeño.

### Un outline nunca omite un símbolo

```console
$ pn-ultramemory outline crates/codec/src/tokens.rs
file:
  path: crates/codec/src/tokens.rs
  lines: 274
  symbols: 21
  detail: documented
  tokens: 1061
  whole_file_tokens: 2669
  saved: 0.602
```

Todos los símbolos que declara el fichero, en orden, con su anidamiento. Cuando el presupuesto no
da, baja el **detalle** — documentación, luego firmas, hasta nombres pelados — y la lista de símbolos
queda entera, porque un esqueleto al que le faltan tres funciones se lee como *no están ahí*.

También dice cuándo el fichero es tan corto que **sale más barato leerlo entero**, en vez de cobrarte
tokens por una respuesta peor que `cat`.

### El sobre epistémico

`impact` nunca dice «seguro». Dice una de tres cosas:

| | |
|---|---|
| **`exact`** | El conjunto está completo |
| **`lower-bound`** | Al menos estos; puede haber más |
| **`unknown`** | No se encontró ningún llamador y el símbolo es público — lo que **no** significa que nadie lo use |

Cada arista lleva cómo de seguro estaba el indexador: `Guess`, `Heuristic`, `Resolved`, `Exact`.

<p align="center">
  <img src="assets/diagrams/memory.svg" alt="Una memoria se guarda con los hashes del símbolo que describe; cuando ese símbolo cambia, la memoria se marca obsoleta en vez de borrarse o darse por buena" width="760">
</p>

### La regla que gobierna la memoria

> **Que algo se repita sube la probabilidad de que se RECUPERE. Nunca sube la probabilidad de que
> sea VERDAD.**

Sin ella, un agente repitiendo su propio error en bucle fabricaría un hecho, y la herramienta se lo
serviría con confianza a todas las sesiones siguientes. Por eso: dos repeticiones **a menos de 15
minutos cuentan como una**, venga de quien venga; el texto capturado de una herramienta
(`--by tool`) **nunca** corrobora; y ninguna memoria acumula más de **8** corroboraciones.

### La contradicción se comprueba antes que el parecido

Un caso real de este repositorio explica por qué:

| Par | Parecido | Realidad |
|---|---:|---|
| «calibrado contra un tokenizador real, **no** adivinado» vs «**no** calibrado contra un tokenizador real» | **0,838** | Se contradicen |
| «nunca desenvolver en una ruta de petición, devolver un error» vs «nunca desenvolver en una ruta de petición; devolver un error en su lugar» | **0,844** | Coinciden |

**La contradicción se parece menos que el acuerdo.** Ninguna medida de parecido textual puede
distinguirlos, porque negar una frase cambia casi ninguna de sus palabras. Sin la comprobación
estructural, una memoria podía ser reforzada por su propio opuesto.

---

## El grafo

```bash
pn-ultramemory graph --modules --as svg -o grafo.svg
```

Ningún otro grafo de código se dibuja así, y esa es la intención.

- **Una arista es una forma rellena, no una línea.** Dos cúbicas comparten sus puntos de control, así
  que la fibra es ancha donde sale y acaba en punta donde llega. **El afilado lleva la dirección**,
  por eso no hay una sola punta de flecha. Cien puntas de flecha son ruido; cien afilados son
  textura.
- **Las fibras se agrupan en haces por los dos extremos.** El segundo es el que importa: un grafo de
  código no es un árbol que llama hacia fuera, son unos pocos símbolos a los que todo llama *hacia
  dentro*. Agrupar por destino es lo que hace que las fibras converjan sobre el soma.
- **Toda fibra para antes del soma al que apunta.** Ese hueco es la hendidura sináptica, y el
  ensanchamiento en su borde es el terminal.
- **Una fibra toma el color del módulo del que sale**, así una conexión se rastrea por el color.
- **El halo son tres discos planos**, no un filtro de desenfoque: es lo más lento de un dibujo de
  este tamaño, y tres escalones de opacidad ya se leen como luz.

SVG y CSS puros. Sin JavaScript, sin fuentes externas, sin una sola petición. Claro y oscuro siguen
al lector, la paleta es segura para daltonismo (Okabe–Ito), y la misma entrada siempre produce los
mismos bytes.

---

## Funciona con el agente que ya usas

`pn-ultramemory install` se registra allí donde encuentra un agente, escribiendo **solo su propia
entrada** y dejando el resto del fichero byte por byte como estaba.

| | | | |
|---|---|---|---|
| Claude Code | Codex CLI | Cursor | Gemini CLI |
| Windsurf | Zed | Visual Studio Code | opencode |
| **Kiro** | **Trae** | Cline | Crush |
| Amp | | | |

<sub>`uninstall` quita exactamente lo que `install` escribió — la ida y vuelta deja el fichero
idéntico, y eso es lo que comprueban sus 60 pruebas. La ruta de Trae está marcada como *sin
verificar*: se ofrece con `--agents trae` y no se escribe por defecto.</sub>

Cinco herramientas por MCP — **`recall`**, **`outline`**, **`impact`**, **`remember`** y
**`expand`** — y la lista entera cuesta **2.139 bytes** del contexto del agente.

---

## Arquitectura

Hexagonal: el núcleo no sabe nada de SQLite, de tree-sitter ni del sistema de ficheros.

```
          ┌──────────┐   ┌──────────┐   ┌──────────┐
 entrada  │   cli    │   │   mcp    │   │  report  │
          └────┬─────┘   └────┬─────┘   └────┬─────┘
               └──────────────┼──────────────┘
                         ┌────▼─────┐
 casos de uso            │  engine  │   index · recall · outline · remember
                         └────┬─────┘   impact · graph · map · docs · bench
                              │
                         ┌────▼─────┐
 contratos               │   core   │   Storage · Extractor · SourceTree · Clock
                         └────┬─────┘   sin E/S, sin dependencias internas
               ┌──────────────┼──────────────┐
          ┌────▼─────┐   ┌────▼─────┐   ┌────▼─────┐
 adapt.   │  store   │   │  index   │   │  codec   │
          │ (SQLite) │   │(tree-sit)│   │  (TOON)  │
          └──────────┘   └──────────┘   └──────────┘
```

| Crate | Líneas | Qué es |
|---|---:|---|
| `core` | 2.319 | Los contratos. Sin E/S, sin depender de ningún otro crate de aquí |
| `codec` | 1.743 | Estimación de tokens y la mochila multi-resolución |
| `toon` | 5.435 | TOON 4.1 — pasa las **538** pruebas oficiales de conformidad |
| `index` | 13.012 | tree-sitter para 12 lenguajes, más respaldo para todos los demás |
| `store` | 10.802 | SQLite con WAL y FTS5 |
| `engine` | 18.793 | Todos los casos de uso |
| `mcp` | 5.679 | Model Context Protocol por stdio, cinco revisiones |
| `report` | 12.421 | HTML, PDF y Markdown — **cero dependencias** |
| `cli` | 8.034 | La línea de órdenes, hooks, color y el instalador |
| `xtask` | 3.850 | Las guardas de calidad |
| | **83.035** | **1.335 pruebas** |

Analizados con tree-sitter: **Rust, Python, JavaScript, TypeScript, TSX, Go, Java, C, C++, C#, Ruby,
PHP**. Cualquier otro lenguaje se indexa igual con un respaldo léxico, así que nada de tu
repositorio queda invisible.

---

## Lo que no hace

Una herramienta honesta dice lo que no puede hacer.

- **No entiende significado.** Toda comparación de texto es sobre palabras. Una paráfrasis que no
  comparte ninguna no se reconoce como duplicada.
- **Entiende inglés y español.** Un texto en otro idioma se guarda y se recupera bien, pero rara vez
  se reconoce como duplicado de otro en ese idioma.
- **No distingue lo verdadero de lo falso.** Obsoleta significa *el código cambió*, no *la memoria
  es ahora falsa*. Eso lo decide una persona.
- **La detección de contradicciones tiene poca cobertura a propósito.** Captura oposición
  estructural. Se le escapará una que necesite conocer tu dominio.
- **`task_success` no se mide.** Si tu agente resolvió el problema queda fuera de lo observable.

---

## Calidad

Nada se integra si algo de esto no está en verde.

| | |
|---|---|
| **1.335 pruebas** | unitarias, de integración, de propiedades, de conformidad y de clase de coste |
| `cargo clippy --all-targets -- -D warnings` | sin hallazgos |
| `cargo doc -D warnings` | sin hallazgos |
| `cargo deny check` | avisos, prohibiciones, licencias, orígenes |
| **Solo licencias permisivas** | una licencia permitida que ya nadie usa **rompe** la comprobación |
| `#![forbid(unsafe_code)]` | en todos los crates |

Cuatro ratchets vigilan lo que las pruebas no pueden. Cada uno tiene una línea base que **solo puede
encoger**, y una entrada que deja de coincidir con algo **rompe la compilación**, así que un guard no
se puede desactivar en silencio:

| Ratchet | Qué sostiene |
|---|---|
| `headers` | Cada fichero lleva su licencia y su documentación de módulo |
| `refusal` | Cada mensaje de error dice cómo seguir |
| `panics` | El código de biblioteca no entra en pánico |
| `docs` | La primera frase de un elemento público añade algo a su nombre |

---

## Documentación

| | |
|---|---|
| [docs/architecture.md](docs/architecture.md) | Las capas y por qué están separadas |
| [docs/memory.md](docs/memory.md) | Qué se guarda, cómo se detectan duplicados y contradicciones |
| [docs/formats.md](docs/formats.md) | TOON, cápsulas y todas las formas de salida |
| [docs/benchmark.md](docs/benchmark.md) | Cómo se producen los números de arriba |
| [docs/quality.md](docs/quality.md) | Cada guarda, y qué **no** demuestra |
| [docs/glossary.md](docs/glossary.md) | Las palabras que este proyecto usa con precisión |

**Para usarlo:** el [wiki](https://github.com/paulnewmandev/pn-ultramemory/wiki) — instalación, la
primera hora, todas las órdenes, preguntas frecuentes y resolución de problemas.
**Para que lo instale un agente:** dale [AGENTS.md](AGENTS.md).
**Para contribuir:** [CONTRIBUTING-WORKFLOW.md](CONTRIBUTING-WORKFLOW.md).

---

## Licencia y contribuciones

Apache-2.0. Úsalo, cámbialo, véndelo, bifúrcalo — ver [LICENSE](LICENSE) y
[TRADEMARKS.md](TRADEMARKS.md).

[CONTRIBUTING.md](CONTRIBUTING.md) · [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) ·
[SECURITY.md](SECURITY.md) · [SUPPORT.md](SUPPORT.md) · [GOVERNANCE.md](GOVERNANCE.md)

<p align="center"><sub>
Hecho por <a href="https://github.com/paulnewmandev">Paul Newman</a>. Sin telemetría. Sin cuenta.
Sin red. Gratis, y así se queda.
</sub></p>
