"""Notebook-shaped lazy project workspace example.

Run with a synthetic/local ``.pproj`` path; this file contains no dataset.
"""

import petekio


project = petekio.Project.load("field.pproj")

# Inspect the catalog without opening a browser or requiring petekTools.
session = project.view(settings=petekio.ViewSettings(serve=False))
print(session.tree())

# Logs are explicit; catalog startup still gathers no samples.
workspace = project.view(
    selection={"surfaces": ["Interpretation/"], "wells": True},
    logs=petekio.ViewSpec(curves=("PHIE", "SW"), tops=True),
    tab="map",
)

workspace.save("project-workspace.html", include="visible")
