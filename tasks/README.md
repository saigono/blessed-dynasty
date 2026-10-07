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
| 12 | 12-heirs-neighbours.md | 8b |
| 13 | 13-ui-clarity.md | 12 |
| 14 | 14-war.md | 13 |
| 15 | 15-balance.md | 14 |
| 16 | 16-succession-laws.md | 15 |
| 17 | 17-heirs-legitimacy.md | 16 |
| 17b | 17b-disputes-criterion.md | 17 |
| 18 | 18-influence-graph.md | 17b |
| 19 | 19-institutions.md | 18 |
| 20 | 20-symptoms-chain.md | 19 |
| 21 | 21-transparency-ui.md | 20 |
| 22 | 22-literary-chronicle.md | 21 |
| 23 | 23-content-editor.md | 22 |
| 24 | 24-testament.md | 19, 22 |
| 25 | 25-playtest-02.md | 24 |
| 26 | 26-realms.md | 25 |
| 26b | 26b-playtest-03.md | 26 |
| 26c | 26c-rich-texts.md | 26b |
| 27 | 27-wars-and-breakups.md | 26b |
| 27b | 27b-parallel-batch.md | 26c, 27 |
| 28 | 28-big-map-empire.md | 27b |
| 28b | 28b-succession-laws.md | 28 |
| 29 | 29-old-map-ui.md | 28 |
| 29b | 29b-event-pictures.md | 29 |
| 29c | 29c-map-polish.md | 28b, 29b |
| 30 | 30-texts-and-faith.md | 28b, 29b |
| 30b | 30b-one-faith-people.md | 30 |
| 29d | 29d-playtest-04.md | 29c, 30 |
| 30c | 30c-one-faith-literacy.md | 30b |
| 31 | 31-capital-move.md | 29d, 30b |

После этапа 2 параллельно можно вести 3, 5, 8a, 9, 10a. На соло-разработке реально две-три ветки за раз.

## Запуск

Статусы живут в трекере: https://claude.ai/artifact/SmCxHTyYecE3DouTDo2K5Q

Из Claude Code в корне репозитория:

- «работай над следующей задачей» или «работай над задачей 3»: скилл `work-task` создаёт worktree `../bd-stage-NN` на ветке `stage/NN`, ставит статус «в работе», запускает агента в фоне, по завершении гоняет тесты и ставит «на приёмке».
- «прими задачу 3»: merge в main, тесты, удаление worktree, статус «готово».
- «статус задач»: сводка из трекера.

Вручную то же самое:

```
git worktree add ../bd-stage-00 -b stage/00 main
cd ../bd-stage-00
claude "$(cat tasks/00-skeleton.md)"
```
