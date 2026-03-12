# Clippix — Contexto del proyecto

## Qué es
App de escritorio para Linux. Gestor de portapapeles unificado con captura
de pantalla y editor de anotaciones con blur. Construida con Tauri v2.

## Stack
- Tauri v2
- Rust (backend, src-tauri/)
- React + TypeScript + Vite (frontend, src/)
- Tailwind CSS
- SQLite via rusqlite
- Zustand para estado global

## Reglas generales
- No asumas. Si falta contexto, preguntá antes de implementar.
- Buscá código existente antes de crear algo nuevo.
- Cambios mínimos: no reescribas lo que ya funciona.
- Si ves un problema en mi enfoque, dimelo directamente.
- Explicá siempre por qué elegiste una solución sobre otra.

## Rust (src-tauri/)
- Nunca uses unwrap() en producción, siempre Result<T, E>.
- Separé lógica de negocio de los comandos #[tauri::command].
- Para captura: detectá X11 o Wayland al inicio y usá la implementación correcta.
- SQLite con transacciones para operaciones múltiples.

## Frontend (src/)
- Componentes funcionales únicamente con hooks.
- Llamadas a Tauri siempre via invoke() con tipos definidos en src/types/.
- Canvas nativo para el editor de anotaciones, sin librerías externas.
- PascalCase para componentes, camelCase con prefijo "use" para hooks.

## Tauri v2
- Permisos en capabilities/, no en tauri.conf.json directamente.
- Eventos frontend↔backend via emit/listen, no polling.
- El tray icon debe funcionar sin ventana abierta.

## Feature principal
Editor de anotaciones sobre canvas HTML5: flechas, texto, rectángulos y
blur por selección de región. Es el diferenciador vs Flameshot.