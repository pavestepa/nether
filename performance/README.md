# performance

Сравнение Nether и вручную написанного Rust на парных программах.

```sh
./performance/run.sh 7 --build-runs 3
```

Запускать без других CPU-intensive задач. Требуются Python 3 и Rust toolchain.
`NETHER_RUSTC` задаёт один rustc для обеих сторон. Сборка компилятора выполняется
в release с `--locked --offline` до измерений.

- [bench/](bench/) — пары `name.nr` и `name.rs`.
- [run.py](run.py) — прогрев, чередование сторон, медиана/min/max, проверка
  кодов выхода каждого повтора, время сборки и размер executables.
- [RESULTS.md](RESULTS.md) — опубликованный замер и ограничения сравнения.
- [results/](results/) — сохранённые сырые данные и предыдущий отчёт.

Новые данные сохраняются в `performance/out/results.json`; бинарники в той же
игнорируемой папке. Для публикации нужно скопировать JSON в `results/` и обновить
отчёт. `class_alloc` сохраняет Rust allocation через black_box;
`class_alloc_elidable` разрешает Rust удалить ненужную allocation. Эти результаты
нельзя смешивать при выводах о скорости управления памятью.

Дополнительный kernel probe с полным сравнением u64-результатов и black_box
для входов обеих сторон:

```sh
cargo test -p nether-core --release --test performance --offline -- --ignored --nocapture
```

Он использует один executable, 9 раундов и медиану; не имеет timing threshold
и не служит обещанием производительности для произвольных программ.
