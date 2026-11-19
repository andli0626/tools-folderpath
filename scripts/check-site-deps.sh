#!/usr/bin/env bash
# 发布页依赖自检：确保站点不引入任何境外 / 高风险外部依赖，并且下载全部同源。
# 用法：在仓库根目录执行  ./scripts/check-site-deps.sh
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RISK='fonts\.googleapis|fonts\.gstatic|cdn\.jsdelivr|unpkg\.com|cdnjs\.cloudflare'
RISK+='|google-analytics|googletagmanager|gtag\(|hm\.baidu'
RISK+='|raw\.githubusercontent|codeload\.github|objects\.githubusercontent|avatars\.githubusercontent'
RISK+='|releases\/download|polyfill\.io|staticfile\.org|bootcdn'

fail=0

echo "[1/3] 扫描 HTML / CSS / JS 中的高风险依赖"
while IFS= read -r f; do
  hits=$(grep -nEo "$RISK" "$f" 2>/dev/null || true)
  if [ -n "$hits" ]; then
    echo "  [风险] $f"
    echo "$hits" | sed 's/^/      /'
    fail=1
  fi
done < <(find . -path ./.git -prune -o -type f \( -name '*.html' -o -name '*.css' -o -name '*.js' \) -print)

echo "[2/3] 检查 index.html 的下载链接是否全部同源"
if [ -f index.html ]; then
  ext=$(grep -oE 'href="[^"]*(releases/download|github\.com/[^"]*/releases)[^"]*"' index.html || true)
  if [ -n "$ext" ]; then
    echo "  [风险] 发现 Release 下载链接："
    echo "$ext" | sed 's/^/      /'
    fail=1
  fi
else
  echo "  [失败] 根目录缺少 index.html"
  fail=1
fi

echo "[3/3] 检查 .nojekyll 与下载文件"
if [ ! -e .nojekyll ]; then
  echo "  [风险] 根目录缺少 .nojekyll（下划线/点号目录会 404）"
  fail=1
fi
for f in downloads/*.dmg downloads/*.zip; do
  [ -e "$f" ] || continue
  echo "  [就绪] $f ($(du -h "$f" | cut -f1))"
done

if [ "$fail" -eq 0 ]; then
  echo "[通过] 未发现高风险外部依赖，下载走同源路径"
else
  echo "[失败] 请移除上述引用后再发布"
  exit 1
fi
