"""Post-build smoke test for a frankenpandas wheel (Wheels workflow).

Run with the built wheel installed and EXPECTED_VERSION set to the Cargo
package version. Fails when the wheel reports another version (the 0.3.0 wheel
said 0.2.0) or when read_csv of a real file comes back empty (the old
fabricated-empty-frame failure mode). Expected values are what pandas 2.2.3
returns for the same calls.
(br-frankenpandas-rc0923-epic-buildable-everywhere-0zz8y.3)
"""

import os
import tempfile

import frankenpandas as fpd

expected_version = os.environ["EXPECTED_VERSION"]
print("frankenpandas", fpd.__version__, "expected", expected_version)
assert fpd.__version__ == expected_version, fpd.__version__

path = os.path.join(tempfile.mkdtemp(), "smoke.csv")
with open(path, "w") as fh:
    fh.write("k,v\na,1\nb,2\na,3\n")
df = fpd.read_csv(path)
assert df.shape == (3, 2), df.shape

# pandas: read_csv(...).groupby('k').sum().to_csv() == 'k,v\na,4\nb,2\n'
grouped = df.groupby("k").sum()
assert grouped.to_csv() == "k,v\na,4\nb,2\n", repr(grouped.to_csv())

# pandas: df.merge(DataFrame({'k': ['a', 'b'], 'w': [10, 20]}), on='k').shape == (3, 3)
merged = df.merge(fpd.DataFrame({"k": ["a", "b"], "w": [10, 20]}), on="k")
assert merged.shape == (3, 3), merged.shape

print("wheel smoke OK")
