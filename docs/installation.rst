Installation
============

Requirements
------------

* Python 3.8+ (including Python 3.13 and 3.14)
* Rust 1.70+ (for building from source)

Installing from PyPI
--------------------

The easiest way to install rapsqlite is using pip:

.. code-block:: bash

   pip install rapsqlite

Building from Source
--------------------

To build rapsqlite from source, you'll need Rust installed:

1. Install Rust (if not already installed):

   .. code-block:: bash

      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

2. Install maturin (Python-Rust build tool):

   .. code-block:: bash

      pip install maturin

3. Clone the repository:

   .. code-block:: bash

      git clone https://github.com/eddiethedean/rapsqlite.git
      cd rapsqlite

4. Build and install in development mode:

   .. code-block:: bash

      maturin develop

   Or install in production mode:

   .. code-block:: bash

      maturin build
      pip install target/wheels/rapsqlite-*.whl

Platform-Specific Notes
-----------------------

**macOS**: Works on both Intel and Apple Silicon (arm64).

**Linux**: Works on x86_64 and aarch64 architectures.

**Windows**: Supported on x86_64 and aarch64.

Verifying Installation
----------------------

After installation, verify that rapsqlite is working:

.. code-block:: python

   import asyncio
   from rapsqlite import connect

   async def test():
       async with connect(":memory:") as conn:
           await conn.execute("CREATE TABLE test (id INTEGER)")
           await conn.execute("INSERT INTO test VALUES (1)")
           rows = await conn.fetch_all("SELECT * FROM test")
           print(rows)  # [[1]]

   asyncio.run(test())

If this runs without errors, rapsqlite is installed correctly!
