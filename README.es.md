<p align="center">
  <img src="assets/logo/banner.svg" alt="pn-ultramemory" width="380">
</p>

<h1 align="center">pn-ultramemory</h1>

<p align="center">
  <b>Un segundo cerebro para tu código, compartido entre tú y tu agente de programación.</b><br>
  Indexa el repositorio en un grafo, responde preguntas con unos cientos de tokens en lugar de
  archivos enteros, recuerda decisiones ancladas al código y te deja recorrerlo todo en 3D.
</p>

<p align="center">
  <a href="README.md">🇬🇧 English</a> ·
  <a href="README.es.md">🇪🇸 Español</a>
</p>

<p align="center">
  <a href="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/ci.yml/badge.svg?branch=main"></a>
  <a href="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/guards.yml"><img alt="Guards" src="https://github.com/paulnewmandev/pn-ultramemory/actions/workflows/guards.yml/badge.svg?branch=main"></a>
  <a href="LICENSE"><img alt="Licencia: Apache-2.0" src="https://img.shields.io/badge/licencia-Apache--2.0-blue"></a>
  <img alt="Hecho con Rust" src="https://img.shields.io/badge/hecho%20con-Rust-000000?logo=rust&logoColor=white">
  <img alt="1368 pruebas pasando" src="https://img.shields.io/badge/pruebas-1368%20pasando-success">
  <img alt="Red: nunca" src="https://img.shields.io/badge/red-nunca-success">
  <img alt="Telemetría: ninguna" src="https://img.shields.io/badge/telemetr%C3%ADa-ninguna-success">
  <img alt="Estado: proyecto nuevo" src="https://img.shields.io/badge/estado-proyecto%20nuevo-orange">
</p>

<p align="center">
  <img src="assets/screenshots/brain-overview.jpg" alt="Este repositorio dibujado como un cerebro: miles de partículas luminosas, una por símbolo, coloreadas por carpeta, con las pruebas en el cerebelo y anillos dorados para las memorias" width="860">
</p>

---

## Por qué existe

Un agente de programación aprende tu código leyendo archivos enteros. Casi todo lo que lee no es la
respuesta, pagas cada token, y todo lo que aprendió se pierde al cerrar la sesión.

pn-ultramemory guarda ese conocimiento en disco:

- **Un grafo del código.** Cada función, método, clase y tipo, y quién llama y usa a quién, cada
  relación marcada con la seguridad que tiene el indexador.
- **Respuestas, no archivos.** Una pregunta devuelve una *cápsula*: los símbolos que importan, cada
  uno con el nivel de detalle que cabe, dentro de un presupuesto de tokens que tú fijas.
- **Memorias que no pueden mentir en silencio.** Las decisiones y lecciones quedan ancladas al
  código que describen. Cuando ese código cambia, la memoria se marca como obsoleta en vez de
  seguir dándose por buena.
- **Un cerebro que se recorre.** El mismo grafo y las mismas memorias, dibujados en 3D en tu
  navegador, para que una persona vea la forma del código y le pase cualquier parte a un agente.

Un solo binario. Nunca abre una conexión de red, no escribe nada dentro de tu repositorio y no
cuesta nada.

<p align="center">
  <img src="assets/diagrams/why.svg" alt="En este repositorio una pregunta cuesta 16.520 tokens leyendo archivos y encuentra el código correcto el 84 % de las veces; una cápsula cuesta 464 tokens y lo encuentra el 100 % de las veces" width="760">
</p>

---

## Empieza en dos minutos

```bash
# 1. Instala (macOS y Linux): el binario correcto para tu máquina, verificado con su SHA-256
curl -fsSL https://raw.githubusercontent.com/paulnewmandev/pn-ultramemory/main/install.sh | sh

# 2. Desde la raíz de tu proyecto, construye el grafo
pn-ultramemory index

# 3. Conecta tu agente y reinícialo (un servidor MCP solo se lee al arrancar)
pn-ultramemory install --agents claude-code     # o cursor, codex, gemini, windsurf, zed, …

# 4. Míralo
pn-ultramemory brain --lang es
```

El instalador deja el binario en `~/.local/bin` y, si esa carpeta aún no está en tu `PATH`, te
muestra la línea exacta que debes añadir. No necesita permisos de administrador y no cambia nada más.

- **Windows:** descarga `pn-ultramemory-x86_64-pc-windows-msvc.zip` de la
  [última versión](https://github.com/paulnewmandev/pn-ultramemory/releases/latest), descomprímelo
  y pon `pn-ultramemory.exe` en tu `PATH`.
- **Cualquier otro sistema** (Linux en ARM, por ejemplo): `cargo build --release` con Rust
  instalado.
- **Compruébalo:** `pn-ultramemory doctor` revisa cada paso y te dice la orden que arregla lo que
  falte.
- **Que lo haga un agente:** pásale [AGENTS.md](AGENTS.md), escrito para que un modelo lo siga paso
  a paso.

---

## Lo que recibe tu agente

Nueve herramientas por Model Context Protocol. La lista completa cuesta unos mil tokens, que se
leen una vez por sesión.

| Herramienta | Responde | En lugar de |
|---|---|---|
| **`brief`** | Qué es este proyecto: tamaño, módulos, símbolos más usados, cada decisión registrada | Leer un README y adivinar |
| `recall` | Qué código importa para esta pregunta, dentro de un presupuesto | Leer varios archivos |
| `outline` | Todo lo que declara un archivo, por una fracción de sus tokens | Leer el archivo entero |
| `expand` | El código exacto de un símbolo | Leer lo que lo rodea |
| `impact` | Qué depende de esto, y con cuánta seguridad | Buscar el nombre con grep |
| `map` | Qué archivos hay y qué contienen | Listar el árbol |
| `remember` | Guarda esta decisión, anclada al código | Un comentario que nadie lee |
| `memories` | Qué sabemos ya, y qué quedó obsoleto | Volver a preguntar |
| `feedback` | Esa respuesta sirvió, o no | Nada |

<p align="center">
  <img src="assets/diagrams/session.svg" alt="Una sesión nueva llama a brief para conocer el proyecto, a recall y outline para trabajar, a remember para guardar una decisión y a feedback para decir qué sirvió; el grafo en disco sobrevive a la sesión" width="760">
</p>

Una sesión empieza con `brief`, pregunta con `recall` y un presupuesto, escribe `remember` cuando se
decide algo y envía `feedback` para que la próxima sesión ordene mejor. El grafo y las memorias
sobreviven a la sesión: esa es la idea.

El agente recibe además dos *hooks* pequeños: una línea al iniciar la sesión diciendo que el
repositorio está indexado y, como mucho una vez por sesión, una sugerencia de usar `recall` cuando
va a buscar en todo el repositorio o a leer un archivo muy grande. Un hook nunca bloquea nada.
`PN_ULTRAMEMORY_NO_HOOKS=1` los apaga.

---

## El cerebro

```bash
pn-ultramemory brain --lang es
```

<p align="center">
  <img src="assets/screenshots/brain-symbol.jpg" alt="Un símbolo abierto en la vista del cerebro: sus fibras resaltadas, pulsos que viajan por ellas y un panel con su firma, su documentación, una memoria anclada, dos llamadores y trece llamados" width="860">
</p>

- **La forma significa algo.** Cada carpeta es una región de la corteza, en pares espejados sobre
  los dos hemisferios, la más grande primero. Las pruebas viven en el cerebelo. Las fibras corren
  por debajo como la sustancia blanca y se desvanecen de donde salen a donde llegan, así la
  dirección se ve sin flechas. Los pulsos viajan de quien llama a quien es llamado.
- **Busca y lee.** `/` busca entre símbolos, rutas, documentación y memorias. Al abrir una partícula
  ves su firma, su documentación, quién la llama, a quién llama y cada memoria anclada a ella; cada
  uno es un enlace a la siguiente partícula, como las notas enlazadas de un cuaderno.
- **Pásaselo a un agente.** *Copiar contexto para un agente* pone en el portapapeles el símbolo, su
  ubicación, quién lo llama, a quién llama y sus memorias, con las órdenes para seguir: unos cientos
  de tokens que orientan a un modelo más rápido que cualquier archivo.
- **Sellado.** Un solo HTML en el directorio de datos, WebGL puro sin librerías. Su
  Content-Security-Policy no nombra ningún origen para conexiones, imágenes, fuentes ni marcos, así
  que el propio navegador rechaza cualquier petición que la página intente.

Se abre en el navegador cuando lo ejecutas en una terminal; `--no-open` solo lo escribe y `-o` lo
pone en otro lugar. En repositorios muy grandes se quedan los 6000 símbolos de los que más se
depende (`--max-nodes`), que se dibujan con fluidez en un portátil. Este repositorio, con 4890
símbolos, se genera en un cuarto de segundo.

---

## Lo que ahorra, medido

`pn-ultramemory bench` toma 100 símbolos documentados, convierte cada uno en una pregunta y compara
la cápsula con lo que hace un agente sin índice: leer los tres archivos que mejor puntúa una
búsqueda por palabras. En este repositorio (274 archivos, 5232 símbolos):

| Pregunta construida a partir de | Presupuesto | Encuentra el código | Tokens | Leyendo archivos | Ahorro |
|---|---:|---:|---:|---:|---:|
| su documentación | 500 | **100 %** | 464 | 16.520 · lo encuentra el 84 % | **97,2 %** |
| su documentación | 1000 | **100 %** | 645 | 16.520 | **96,1 %** |
| su documentación | 2000 | **100 %** | 1366 | 16.520 | **91,7 %** |
| su nombre | 500 | **100 %** | 446 | 15.438 · lo encuentra el 62 % | **97,1 %** |
| su nombre | 1000 | **99 %** | 672 | 15.438 | **95,7 %** |

En una aplicación Laravel de 541 archivos, el mismo benchmark da un 100 % a partir de la
documentación con un 95 % menos de tokens, y entre un 85 y un 93 % a partir del nombre, frente al
66 % de leer archivos.

**Lee estas cifras por lo que son.** Las preguntas salen de la propia documentación de los
símbolos, así que miden encontrar algo conocido, no resolver una tarea, y la herramienta lo dice en
su salida. Las preguntas libres funcionan bien cuando usan las palabras del código ("validate coupon
discount on order" encuentra primero `CouponService::validate`) y peor cuando no comparten ninguna.
Las preguntas en español sobre código escrito en inglés se apoyan en un glosario incorporado de
vocabulario de programación y de negocio: "¿cómo se estima el número de tokens?" encuentra
`estimate_tokens`, y "dividir la cuenta entre clientes" encuentra `BillSplitService`. Las palabras
fuera del glosario, y los demás idiomas, se buscan tal cual. Mide tu propio repositorio con `pn-ultramemory bench`: el ahorro es lo que habría costado leer
archivos enteros, así que un proyecto pequeño ahorra menos.

| Velocidad, este repositorio, portátil Apple silicon | |
|---|---|
| Índice completo desde cero | **~430 ms** |
| Reindexar tras cambiar un archivo | **~200 ms** |
| Reindexar sin cambios | **12 ms** |
| Un `recall` | **5 ms** de mediana, 6,5 ms en el percentil 95 |

---

## Cómo funciona

<p align="center">
  <img src="assets/diagrams/how.svg" alt="El repositorio se indexa en un grafo; una pregunta encuentra semillas, se recorre el grafo, los candidatos se ordenan y se empaquetan en un presupuesto, y la cápsula se mide antes de devolverla" width="760">
</p>

**Un grafo que dice cuán seguro está.** Doce lenguajes se analizan con tree-sitter (Rust, Python,
JavaScript, TypeScript, TSX, Go, Java, C, C++, C#, Ruby, PHP); cualquier otro se indexa con una
pasada léxica, así que nada queda invisible. Cada arista lleva una confianza: `Exact`, `Resolved`,
`Heuristic` o `Guess`.

**Las llamadas se leen con su receptor.** El nombre de un método solo no es prueba suficiente:
`$request->validate()` en un controlador de Laravel no es una llamada a tu único
`CouponService::validate`, ni `items.is_empty()` a tu único `is_empty`. Una llamada hecha sobre un
receptor que no apunta al candidato, ni por su tipo, ni por su archivo, ni por su directorio, es solo
un `Guess`, y por eso queda fuera de `impact`, de `brief` y del cerebro por defecto. El receptor
también ayuda: `engine.recall()` se resuelve a `Engine::recall` entre varios métodos `recall`.

**Una respuesta es un problema de empaquetado.** Cada símbolo puede mostrarse en cinco niveles:
nombre, firma, resumen, lo que llama, o el código completo. `recall` elige un nivel por símbolo para
dar el mayor valor dentro del presupuesto (una mochila de elección múltiple, comprobada contra un
solucionador exacto), luego mide la cápsula impresa y la recorta hasta que cabe. Los tres símbolos
más relevantes conservan al menos su firma mientras quede otra cosa que recortar, y un presupuesto
ajustado baja el detalle en lugar de quitar respuestas.

<p align="center">
  <img src="assets/diagrams/levels.svg" alt="Cinco niveles de detalle para un símbolo, desde solo su nombre hasta su código completo, cada uno más caro que el anterior" width="760">
</p>

**`impact` nunca dice "seguro".** Responde `exact` (el conjunto está completo), `lower-bound` (al
menos estos) o `unknown` (no encontró llamadores y el símbolo es público, lo que no significa que
nadie lo use).

**Las memorias siguen una regla.** *Que una memoria se repita aumenta la probabilidad de recuperarla,
nunca la de que sea verdad.* Las repeticiones en menos de quince minutos cuentan como una, el texto
capturado de una herramienta nunca corrobora y ninguna memoria acumula más de ocho. Las
contradicciones se comprueban antes que el parecido, porque negar una frase cambia casi ninguna de
sus palabras, y se informan, nunca se fusionan. El inglés y el español se leen bien.

<p align="center">
  <img src="assets/diagrams/memory.svg" alt="Una memoria se guarda con hashes del símbolo que describe; cuando ese símbolo cambia la memoria se marca como obsoleta en lugar de borrarse o darse por buena" width="760">
</p>

---

## Órdenes

| Orden | Qué hace |
|---|---|
| `index` | Construye o actualiza el grafo. Solo se vuelven a leer los archivos cuyo contenido cambió |
| `brief` · `recall` · `outline` · `expand` · `impact` · `map` | Las seis formas de leer, como arriba |
| `brain` | El repositorio como un cerebro 3D en el navegador |
| `graph` | Exporta el grafo como Mermaid, DOT, SVG o JSON |
| `remember` · `memories` · `reanchor` · `forget` | Escribe, lista, confirma o descarta memorias |
| `feedback` · `learn status\|why\|reset` | Dile qué sirvió, y revisa lo que aprendió |
| `report` | Un informe en HTML, PDF o Markdown, en inglés o español |
| `docs gaps\|apply\|build` | Encuentra código sin documentar, aplica documentación, genera una referencia |
| `stats` · `bench` | Cifras del índice, y el benchmark de arriba sobre tu repositorio |
| `install` · `uninstall` · `doctor` · `serve` · `mcp-config` | Conecta agentes de forma exactamente reversible y revisa la instalación |
| `toon encode\|decode` · `completions` | Conversión de formato y autocompletado de la shell |

La salida es TOON por defecto, un formato de tablas compacto que cuesta alrededor de un 20 % menos
de tokens que JSON; `-f json` es para programas y `-f text` para personas. Cada flag está en la
[wiki](https://github.com/paulnewmandev/pn-ultramemory/wiki/Commands) y en `--help`.

`install` funciona con Claude Code, Codex CLI, Cursor, Gemini CLI, Windsurf, Zed, Visual Studio
Code, opencode, Kiro, Trae, Cline, Crush y Amp. Nombra el tuyo con `--agents`; sin él, se configuran
todos los que encuentre. Escribe solo su propia entrada, `--dry-run` muestra el cambio antes y
`uninstall` deja el archivo exactamente como estaba.

---

## Qué queda dónde

| | |
|---|---|
| Tu código | Se lee, nunca se envía a ningún sitio. El binario no incluye código de red, y la CI ejecuta todas las pruebas en un espacio de red sin salida |
| El índice, las memorias y los contadores | Tu directorio de datos de usuario, una base de datos por proyecto |
| Tu repositorio | Intacto salvo que lo pidas: `docs apply` escribe la documentación que le das, y `report` escribe en el directorio actual si no pasas `-o` |
| La página del cerebro | En el directorio de datos; su política de seguridad le prohíbe cualquier petición |

---

## Lo que no hace

- **No entiende el significado.** Toda comparación es por palabras: una paráfrasis que no comparte
  palabras con el código o con una memoria no se reconoce.
- **Solo traduce del español, y solo su vocabulario de programación.** Un glosario de unas 180
  palabras (*validar*, *pedido*, *factura*, …) añade a una pregunta en español las palabras en
  inglés que usa el código; todo lo demás se busca tal cual, y una palabra pegada dentro de un
  identificador (`recalculate`) no se encuentra por una de sus partes.
- **No distingue lo verdadero de lo falso.** *Obsoleta* significa que el código cambió, no que la
  memoria sea falsa; eso lo decide una persona con `reanchor` o `forget`.
- **No sabe si tu agente lo logró.** `bench` informa el éxito de la tarea como no observable en
  lugar de inventar un número.
- **Es nuevo.** Un autor y pocos usuarios todavía. Tómalo como algo para probar, mídelo en tu propio
  código y reporta lo que falle.

---

## Para quien contribuye

```
           cli      mcp      report          entradas: terminal, agentes, archivos
              \      |      /
                  engine                     todos los casos de uso
                    |
                   core                      solo contratos: sin E/S
              /     |      \
          store   index   codec              SQLite · tree-sitter · empaquetado de tokens
```

Hexagonal: el núcleo no sabe nada de SQLite, tree-sitter ni el sistema de archivos. Nada se fusiona
sin que pasen el formato, `clippy -D warnings`, la documentación, 1368 pruebas y `cargo deny` en
macOS, Linux y Windows, además de cuatro trinquetes que solo pueden encogerse: cabeceras de
licencia, mensajes de error que indican cómo seguir, ningún pánico en código de librería y
documentación que dice más que el nombre del elemento. Consulta [CONTRIBUTING.md](CONTRIBUTING.md),
[CONTRIBUTING-WORKFLOW.md](CONTRIBUTING-WORKFLOW.md) y [AI_POLICY.md](AI_POLICY.md).

| | |
|---|---|
| [Wiki](https://github.com/paulnewmandev/pn-ultramemory/wiki) | Instalación, primera hora, cada orden, el cerebro, herramientas MCP, preguntas frecuentes, problemas |
| [docs/architecture.md](docs/architecture.md) | Las capas y por qué están separadas |
| [docs/memory.md](docs/memory.md) | Qué se guarda y cómo se encuentran duplicados y contradicciones |
| [docs/formats.md](docs/formats.md) | TOON, cápsulas y cada formato de salida |
| [docs/benchmark.md](docs/benchmark.md) | Cómo se obtienen las cifras de arriba |
| [docs/quality.md](docs/quality.md) | Cada guarda, y lo que cada una no demuestra |
| [CHANGELOG.md](CHANGELOG.md) | Qué cambió, versión a versión |

---

Apache-2.0: úsalo, modifícalo, véndelo, haz un fork. Consulta [LICENSE](LICENSE) y
[TRADEMARKS.md](TRADEMARKS.md). [Código de conducta](CODE_OF_CONDUCT.md) ·
[Seguridad](SECURITY.md) · [Soporte](SUPPORT.md) · [Gobierno](GOVERNANCE.md)

<p align="center"><sub>
Hecho por <a href="https://github.com/paulnewmandev">Paul Newman</a>. Sin telemetría, sin cuenta,
sin red. Gratis, y seguirá siéndolo.
</sub></p>
