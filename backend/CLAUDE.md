# CLAUDE.md

## Auto-learned Rules

<!-- claude-evolve:managed-start -->

<!-- claude-evolve:rule id=r_mwufjawz_ibsu score=5.1 created=2026-11-24 source=observation complexity=simple -->
- Při ověřování chování démona/sledování souborů nejdřív spusť krátký foreground zkušební běh (pár sekund), teprve pak dlouhý běh s uloženým výstupem do JSON a ten vyhodnoť skriptem.
<!-- /claude-evolve:rule -->

<!-- claude-evolve:rule id=r_mwufyi83_mi2p score=5.9 created=2026-11-24 source=observation complexity=simple -->
- Při diagnostice chování démona nejdřív zkoreluj výstup sondy (probe-out JSON), daemon.log a ledger.json přes společné časové okno a teprve poté čti zdrojový kód (surface/files.rs), který chování způsobuje.
<!-- /claude-evolve:rule -->

<!-- claude-evolve:managed-end -->
