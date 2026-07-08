from pathlib import Path

import pytest

import petekio


FIXTURES = Path(__file__).resolve().parents[2] / "tests" / "fixtures"


def _project(*, aliases=None):
    return petekio.Project.import_data(FIXTURES / "wells_petro", aliases=aliases)


def test_project_wells_logs_returns_lazy_namespace_and_channels():
    logs = _project().wells.logs

    assert isinstance(logs, petekio.Logs)
    assert isinstance(logs.PHIE, petekio.LogChannel)
    assert logs.PHIE.to_dict() == {
        "kind": "log_channel",
        "mnemonic": "PHI",
        "requested": "PHIE",
    }
    assert logs["PHIE"].to_dict() == logs.PHIE.to_dict()
    assert logs.NTG.to_dict()["mnemonic"] == "NTG"
    assert "PHIE" in logs


def test_project_logs_remains_compatibility_alias():
    project = _project()

    assert project.logs.PHIE.to_dict() == project.wells.logs.PHIE.to_dict()


def test_log_channel_callable_filter_matches_where():
    logs = _project().wells.logs
    predicate = logs.NTG > 0.50

    assert logs.PHIE(predicate).to_dict() == logs.PHIE.where(predicate).to_dict()
    assert logs.PHIE(predicate).to_dict() == {
        "kind": "log_channel",
        "mnemonic": "PHI",
        "requested": "PHIE",
        "filter": {
            "kind": "log_predicate",
            "op": ">",
            "operands": [
                {"kind": "log_channel", "mnemonic": "NTG", "requested": "NTG"},
                {"kind": "scalar", "value": 0.5},
            ],
        },
    }


def test_log_predicates_support_comparisons_and_composition():
    logs = _project().wells.logs
    predicate = ((logs.PHIE >= 0.10) & (logs.NTG <= 0.75)) | ~(logs.SW.is_null())

    assert predicate.to_dict() == {
        "kind": "log_predicate",
        "op": "or",
        "operands": [
            {
                "kind": "log_predicate",
                "op": "and",
                "operands": [
                    {
                        "kind": "log_predicate",
                        "op": ">=",
                        "operands": [
                            {"kind": "log_channel", "mnemonic": "PHI", "requested": "PHIE"},
                            {"kind": "scalar", "value": 0.10},
                        ],
                    },
                    {
                        "kind": "log_predicate",
                        "op": "<=",
                        "operands": [
                            {"kind": "log_channel", "mnemonic": "NTG", "requested": "NTG"},
                            {"kind": "scalar", "value": 0.75},
                        ],
                    },
                ],
            },
            {
                "kind": "log_predicate",
                "op": "not",
                "operands": [
                    {
                        "kind": "log_predicate",
                        "op": "is_null",
                        "operands": [
                            {"kind": "log_channel", "mnemonic": "SUWI", "requested": "SW"}
                        ],
                    }
                ],
            },
        ],
    }


def test_log_predicates_support_channel_to_channel_comparison():
    logs = _project().wells.logs

    assert (logs.PHIE != logs.NTG).to_dict() == {
        "kind": "log_predicate",
        "op": "!=",
        "operands": [
            {"kind": "log_channel", "mnemonic": "PHI", "requested": "PHIE"},
            {"kind": "log_channel", "mnemonic": "NTG", "requested": "NTG"},
        ],
    }


def test_project_aliases_resolve_to_loaded_canonical_mnemonics():
    project = _project(
        aliases={
            "por": ["PHI", "PHIE"],
            "sw": ["SUWI", "SW"],
            "NetSand": ["NTG", "NETSAND"],
        }
    )
    logs = project.wells.logs

    assert logs.names() == ["NetSand", "por", "sw"]
    assert logs.PHIE.to_dict() == {
        "kind": "log_channel",
        "mnemonic": "por",
        "requested": "PHIE",
    }
    assert logs.NetSand.to_dict() == {
        "kind": "log_channel",
        "mnemonic": "NetSand",
        "requested": "NetSand",
    }
    assert logs["NETSAND"].to_dict()["mnemonic"] == "NetSand"


def test_project_resolves_filtered_log_expression_to_positioned_well_logs():
    project = _project(
        aliases={
            "PHIE": ["PHI", "PHIE"],
            "NetSand": ["NTG", "NETSAND"],
        }
    )
    logs = project.wells.logs

    wells = project.resolve_log_expression(logs.PHIE(logs.NetSand >= 0.50))

    assert len(wells) == 1
    assert wells[0]["well_id"] == "15/9-A1"
    assert wells[0]["x"] == 0.0
    assert wells[0]["y"] == 0.0
    assert wells[0]["samples"] == [
        (2400.0, 0.2),
        (2410.0, 0.05),
        (2420.0, 0.2),
        (2430.0, 0.2),
        (2440.0, 0.2),
    ]


def test_project_resolves_serialized_log_expression_source():
    project = _project(
        aliases={
            "PHIE": ["PHI", "PHIE"],
            "NetSand": ["NTG", "NETSAND"],
        }
    )
    source = project.wells.logs.PHIE(project.wells.logs.NetSand >= 0.50).to_dict()

    wells = project.resolve_log_source(source)

    assert wells[0]["samples"] == [
        (2400.0, 0.2),
        (2410.0, 0.05),
        (2420.0, 0.2),
        (2430.0, 0.2),
        (2440.0, 0.2),
    ]


def test_project_log_resolution_cache_returns_defensive_copies():
    project = _project(
        aliases={
            "PHIE": ["PHI", "PHIE"],
            "NetSand": ["NTG", "NETSAND"],
        }
    )
    source = project.wells.logs.PHIE(project.wells.logs.NetSand >= 0.50)

    first = project.resolve_log_expression(source)
    first[0]["samples"].append((9999.0, 0.99))
    second = project.resolve_log_expression(source)

    assert len(project._log_resolution_cache) == 1
    assert second[0]["samples"] == [
        (2400.0, 0.2),
        (2410.0, 0.05),
        (2420.0, 0.2),
        (2430.0, 0.2),
        (2440.0, 0.2),
    ]


def test_project_log_expression_predicates_support_channel_comparison():
    project = _project(
        aliases={
            "PHIE": ["PHI", "PHIE"],
            "NetSand": ["NTG", "NETSAND"],
        }
    )
    logs = project.wells.logs

    wells = project.resolve_log_expression(logs.PHIE(logs.PHIE < logs.NetSand))

    assert [sample[0] for sample in wells[0]["samples"]] == [
        2400.0,
        2410.0,
        2420.0,
        2430.0,
        2440.0,
    ]


def test_unknown_log_mnemonics_fail_loudly():
    logs = _project().wells.logs

    with pytest.raises(KeyError, match="unknown log mnemonic 'NOPE'"):
        logs["NOPE"]
    with pytest.raises(AttributeError, match="unknown log mnemonic 'NOPE'"):
        logs.NOPE


def test_log_expressions_are_not_truthy_python_values():
    logs = _project().wells.logs

    with pytest.raises(TypeError, match="lazy expressions"):
        bool(logs.PHIE)
    with pytest.raises(TypeError, match="lazy expressions"):
        bool(logs.PHIE > 0.1)
