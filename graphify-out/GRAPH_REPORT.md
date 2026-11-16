# Graph Report - karhutla  (2026-11-06)

## Corpus Check
- Corpus is ~4,854 words - fits in a single context window. You may not need a graph.

## Summary
- 110 nodes · 216 edges · 8 communities (5 shown, 3 thin omitted)
- Extraction: 87% EXTRACTED · 13% INFERRED · 0% AMBIGUOUS · INFERRED: 29 edges (avg confidence: 0.86)
- Token cost: 1,200 input · 950 output

## Community Hubs (Navigation)
- SITL, Field Node & Forecasting
- Core Virtual Firmware Pipeline
- HILS Bridge & Smoke Chamber
- Config & Reporting Utilities
- Daily CSV Logging
- MQTT Telemetry & Client I/O
- Architecture & Mode Documentation

## God Nodes (most connected - your core abstractions)
1. `main()` - 23 edges
2. `run()` - 20 edges
3. `Firmware` - 12 edges
4. `main()` - 12 edges
5. `DailyCsvLogger` - 11 edges
6. `LinearTrendForecaster` - 10 edges
7. `PreemptController` - 10 edges
8. `SmokeChamber` - 10 edges
9. `Link` - 7 edges
10. `Kalman2` - 6 edges

## Surprising Connections (you probably didn't know these)
- `main()` --calls--> `Firmware`  [EXTRACTED]
  hils.py → core.py
- `run()` --calls--> `Firmware`  [EXTRACTED]
  sitl.py → core.py
- `main()` --calls--> `DailyCsvLogger`  [EXTRACTED]
  field_node.py → logger.py
- `main()` --calls--> `connect()`  [EXTRACTED]
  field_node.py → mqtt_io.py
- `main()` --calls--> `make_client()`  [EXTRACTED]
  field_node.py → mqtt_io.py

## Import Cycles
- None detected.

## Hyperedges (group relationships)
- **Karhutla Tri-Mode Execution Architecture (SITL, HILS, Riil)** — readme_karhutla_edge_iot, readme_sitl_mode, readme_hils_mode, readme_riil_mode, readme_core_pipeline [EXTRACTED 1.00]

## Communities (8 total, 3 thin omitted)

### Community 0 - "SITL, Field Node & Forecasting"
Cohesion: 0.13
Nodes (6): main(), LinearTrendForecaster, PreemptController, main(), make_plot(), run()

### Community 1 - "Core Virtual Firmware Pipeline"
Cohesion: 0.14
Nodes (6): clamp(), Firmware, HysteresisFSM, Kalman2, Median3, _r0_from_clean()

### Community 2 - "HILS Bridge & Smoke Chamber"
Cohesion: 0.19
Nodes (3): Link, main(), SmokeChamber

### Community 5 - "MQTT Telemetry & Client I/O"
Cohesion: 0.27
Nodes (5): on_msg(), connect(), make_client(), publish_forecast(), publish_telemetry()

### Community 6 - "Architecture & Mode Documentation"
Cohesion: 0.20
Nodes (8): ESP32 Hardware Wiring & Calibration, HILS Mode (Hardware-in-the-Loop), Karhutla Edge-IoT System, MQTT Telemetry and Command Topics, Riil Mode (Field Deployment), SITL Mode (Software-in-the-Loop), matplotlib>=3.5, paho-mqtt>=1.6

## Knowledge Gaps
- **4 isolated node(s):** `HILS Mode (Hardware-in-the-Loop)`, `ESP32 Hardware Wiring & Calibration`, `paho-mqtt>=1.6`, `matplotlib>=3.5`
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 43 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `main()` connect `HILS Bridge & Smoke Chamber` to `SITL, Field Node & Forecasting`, `Core Virtual Firmware Pipeline`, `Config & Reporting Utilities`, `Daily CSV Logging`, `MQTT Telemetry & Client I/O`?**
  _High betweenness centrality (0.154) - this node is a cross-community bridge._
- **Are the 10 inferred relationships involving `main()` (e.g. with `.update()` and `.drain_cmds()`) actually correct?**
  _`main()` has 10 INFERRED edges - model-reasoned connections that need verification._
- **What connects `HILS Mode (Hardware-in-the-Loop)`, `ESP32 Hardware Wiring & Calibration`, `paho-mqtt>=1.6` to the rest of the system?**
  _4 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `SITL, Field Node & Forecasting` be split into smaller, more focused modules?**
  _Cohesion score 0.13230769230769232 - nodes in this community are weakly interconnected._
- **Why does `Firmware` connect `Core Virtual Firmware Pipeline` to `SITL, Field Node & Forecasting`, `HILS Bridge & Smoke Chamber`, `Config & Reporting Utilities`?**
  _High betweenness centrality (0.135) - this node is a cross-community bridge._
- **Are the 10 inferred relationships involving `run()` (e.g. with `.set_mode()` and `.step()`) actually correct?**
  _`run()` has 10 INFERRED edges - model-reasoned connections that need verification._
- **Should `Core Virtual Firmware Pipeline` be split into smaller, more focused modules?**
  _Cohesion score 0.13666666666666666 - nodes in this community are weakly interconnected._