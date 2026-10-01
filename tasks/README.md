# Задачи для агентов

Одна задача на этап. Агент читает CLAUDE.md, DESIGN.md и свой файл задачи.

## Порядок

| Этап | Файл | Ждёт |
| --- | --- | --- |
| 0 | 00-skeleton.md | нет |
| 1 | 01-state.md | 0 |
| 2 | 02-rules.md | 1 |
| 3 | 03-map.md | 2 |
| 8a | 08a-cli-runner.md | 2 |
| 4 | 04-war.md | 2, 3 |
| 5 | 05-reign-end.md | 2 |
| 9 | 09-content.md | 2, растёт вместе с 4, 5, 6 |
| 10a | 10a-ui-reign.md | 2, 3 |
| 6 | 06-simulation.md | 4, 5 |
| 7 | 07-score.md | 6 |
| 8b | 08b-calibration.md | 7, 8a, 9 |
| 10b | 10b-ui-chronicle.md | 7, 10a |
| 11 | 11-url-playtest.md | 8b, 10b |

После этапа 2 параллельно можно вести 3, 5, 8a, 9, 10a. На соло-разработке реально две-три ветки за раз.

## Запуск

```
git worktree add ../bd-stage-00 -b stage/00
cd ../bd-stage-00
claude "$(cat tasks/00-skeleton.md)"
```

После приёмки: merge в main, worktree удалить, следующий этап стартует от нового main.
