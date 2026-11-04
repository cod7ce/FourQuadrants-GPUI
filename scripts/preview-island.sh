#!/bin/bash
# A fresh synthetic store on every run; never opens the user's tasks or reminders.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-dark}"
case "$MODE" in dark|light|system) ;; *) printf 'Usage: bash scripts/preview-island.sh [dark|light|system]\n' >&2; exit 2 ;; esac
cd "$ROOT"
bash scripts/build-native.sh
cargo build --locked
mkdir -p target/visual
PREVIEW_DIR="$(mktemp -d "$ROOT/target/visual/island-demo-XXXXXX")"
python3 -I - "$PREVIEW_DIR/data.json" "$MODE" <<'PY'
import datetime as dt
import json
import sys
from pathlib import Path

today = dt.date.today()
monday = today - dt.timedelta(days=today.weekday())
def stamp(day):
    return dt.datetime.combine(day, dt.time(18, 30)).astimezone().isoformat()
def task(id, title, order, quadrant=1, **fields):
    return dict(id=id, title=title, sortOrder=order, weekStart=monday.isoformat(),
                urgent=quadrant in (0, 2), important=quadrant in (0, 1),
                createdAt=stamp(monday), **fields)
tasks = [
    task('create', '把灵感变成一个小小的作品', 1, links=['https://example.com/island', 'https://example.com/second'], notes='整理今天捕捉到的想法，挑一个最想做的方向。\n先完成一个可以拿在手里的版本，再慢慢打磨细节。', dueDate=stamp(today + dt.timedelta(days=1)), tags=['creative'], progressNotes=[dict(id='update', text='已经选好主题，下一步是做出第一版原型。', createdAt=stamp(today))]),
    task('walk', '走出去，看看秋天', 2, 3, notes='带上相机，在河边散步半小时。\n不用着急回到屏幕前。', tags=['life']),
    task('plan', '为明天留一份清晰的计划', 3, 0, notes='回顾本周的工作，写下三个真正重要的下一步。', dueDate=stamp(today - dt.timedelta(days=1))),
    task('sketch', '画出浮岛展开时的第一帧', 4, parentId='create', notes='让内容从胶囊里自然展开，留足呼吸空间。'),
    task('inspiration', '收集喜欢的交互细节', 5, parentId='create', completed=True, completedAt=stamp(today)),
    task('long-title', '当一个任务包含很多需要解释的上下文，浮岛仍应完整呈现并且可以顺畅滚动', 6, 2, notes='\n'.join(f'第 {i} 条：让信息有序呈现，保持清楚的阅读节奏。' for i in range(1, 20))),
]
for i in range(7, 13):
    tasks.append(task(f'idea-{i}', f'收藏的第 {i} 个小目标', i, i % 4))
doc = dict(schemaVersion=1, tasks=tasks,
    pinnedTaskIds=[t['id'] for t in tasks if not t.get('completed')],
    tags=[dict(id='creative', name='创作', colorHex='#6AA9EF', sortOrder=0), dict(id='life', name='生活', colorHex='#8FBAA4', sortOrder=1)],
    weekNotes=[], settings=dict(theme=sys.argv[2], language='zh', islandEnabled=True,
        taskCompletionSound=False, showInbox=False, windowEffects=dict(transparency=0.1, blurRadius=10),
        parseRules=[dict(id='demo-plain-text', pattern='(?s)^(.+)$', titleGroup=1)]))
Path(sys.argv[1]).write_text(json.dumps(doc, ensure_ascii=False, indent=2) + '\n')
PY
printf 'Island demo: %s\nHover over the black Island at the top edge of the display. Toggle with Cmd+Shift+I.\n' "$PREVIEW_DIR"
exec "$ROOT/target/debug/four-quadrants" --data "$PREVIEW_DIR/data.json"
