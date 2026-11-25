#!/usr/bin/env bash
# Быстрая проверка только затронутых крейтов: rustfmt, check, clippy, test.
# Печатает итоги и строки с ошибками, а не весь вывод cargo, чтобы не засорять контекст агента.
#
#   tools/quick_test.sh                # рабочее дерево + новые файлы относительно HEAD
#   tools/quick_test.sh main           # всё, что отличается от main, включая коммиты ветки
#   tools/quick_test.sh --direct main  # только сами изменённые крейты, без зависимых
#
# Зависимые крейты (те, что тянут изменённые через path-зависимости) тестируются тоже:
# правка в olvin_core без них — самый частый способ пропустить регрессию.
# Корневой Cargo.toml/Cargo.lock не разбирается по крейтам: при его изменении скрипт предупреждает.
#
# Код возврата: 0 — всё зелёное, 1 — что-то упало, 2 — ошибка вызова.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 2

DIRECT=0
if [[ "${1:-}" == "--direct" ]]; then
    DIRECT=1
    shift
fi
BASE="${1:-HEAD}"

if ! git rev-parse --verify -q "$BASE" >/dev/null; then
    echo "quick_test: не знаю ref '$BASE'" >&2
    exit 2
fi

# Изменённые файлы: diff относительно BASE (вместе с рабочим деревом) и неотслеживаемые.
mapfile -t FILES < <(
    { git diff --name-only "$BASE"; git ls-files --others --exclude-standard; } |
        sort -u | grep -v -E '^(\.claude/|target/)' || true
)
if (( ${#FILES[@]} == 0 )); then
    echo "quick_test: нет изменений относительно $BASE"
    exit 0
fi

for f in "${FILES[@]}"; do
    case "$f" in
        Cargo.toml|Cargo.lock|.cargo/config.toml)
            echo "quick_test: тронут корневой манифест ($f), крейты могут не отражать всего; для полной картины cargo test --workspace" ;;
    esac
done

# Файл -> крейт (самый длинный путь к каталогу пакета), плюс обратные зависимости внутри workspace.
PY_CRATES=$(cat <<'PY'
import json, subprocess, sys
from pathlib import Path

direct, files = sys.argv[1] == "1", sys.argv[2:]
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--no-deps"]))
pkgs = {p["name"]: p for p in meta["packages"]}
roots = {n: Path(p["manifest_path"]).parent.resolve() for n, p in pkgs.items()}

changed = set()
for f in files:
    path = Path(f).resolve()
    owners = [n for n, d in roots.items() if path.is_relative_to(d)]
    if owners:
        changed.add(max(owners, key=lambda n: len(roots[n].parts)))

affected = set(changed)
if not direct:
    rev = {}
    for name, p in pkgs.items():
        for dep in p["dependencies"]:
            if dep["name"] in pkgs:
                rev.setdefault(dep["name"], set()).add(name)
    stack = list(changed)
    while stack:
        for user in rev.get(stack.pop(), ()):
            if user not in affected:
                affected.add(user)
                stack.append(user)

print("\n".join(sorted(affected)))
PY
)
CRATES_OUT=$(python3 -c "$PY_CRATES" "$DIRECT" "${FILES[@]}") || exit 2
mapfile -t CRATES < <(printf '%s\n' "$CRATES_OUT" | grep -v '^$' || true)

RS_FILES=()
for f in "${FILES[@]}"; do
    [[ "$f" == *.rs && -f "$f" ]] && RS_FILES+=("$f")
done

if (( ${#CRATES[@]} == 0 )); then
    echo "quick_test: изменены только не-крейтовые файлы, тесты пропущены"
    if (( ${#RS_FILES[@]} > 0 )); then
        rustfmt --check --edition 2024 "${RS_FILES[@]}" >/dev/null 2>&1 && echo "✔ rustfmt" || { echo "✘ rustfmt"; exit 1; }
    fi
    exit 0
fi

PKG_ARGS=()
for c in "${CRATES[@]}"; do
    PKG_ARGS+=(-p "$c")
done
echo "quick_test: крейты (${#CRATES[@]}): ${CRATES[*]}"

# Ускорители из memory-политики: общий sccache и без инкрементальной сборки.
export CARGO_INCREMENTAL=0
# Тесты строгие: паника в системе роняет тест, а не только гасит подсистему.
export OLVIN_FAULTS="${OLVIN_FAULTS:-abort}"
if command -v sccache >/dev/null; then
    export RUSTC_WRAPPER=sccache
    export SCCACHE_DIR="$PWD/target/sccache"
fi
# Рендер без GPU на llvmpipe, как в CI.
MESA_JSON=/usr/share/glvnd/egl_vendor.d/50_mesa.json
if [[ -f "$MESA_JSON" ]]; then
    export __EGL_VENDOR_LIBRARY_FILENAMES="$MESA_JSON" WGPU_BACKEND=gl
fi

FAIL=0

# run <название> <команда...>: молчит об успехе, при провале показывает только строки с ошибками.
run() {
    local name="$1"
    shift
    local out st passed
    out="$("$@" 2>&1)"
    st=$?
    if (( st == 0 )); then
        passed=$(printf '%s\n' "$out" | grep -oE '[0-9]+ passed' | awk '{ s += $1 } END { print s + 0 }')
        if [[ "$out" == *"test result"* ]]; then
            echo "✔ $name ($passed тестов прошло)"
        else
            echo "✔ $name"
        fi
    else
        echo "✘ $name"
        printf '%s\n' "$out" |
            grep -E 'error|FAILED|panicked|failures:|test result|-->|Diff in|warning: unused' |
            head -n 80
        FAIL=1
    fi
}

if (( ${#RS_FILES[@]} > 0 )); then
    run "rustfmt" rustfmt --check --edition 2024 "${RS_FILES[@]}"
fi
run "check" cargo check "${PKG_ARGS[@]}" --all-targets --locked
run "clippy" cargo clippy "${PKG_ARGS[@]}" --all-targets --locked -- -D warnings
run "test" cargo test "${PKG_ARGS[@]}" --locked --no-fail-fast

if (( FAIL == 0 )); then
    echo "quick_test: всё зелёное"
else
    echo "quick_test: есть падения, полный вывод: cargo test ${PKG_ARGS[*]} --locked"
fi
exit "$FAIL"
