"""Local proxy regression tests; no provider credentials or public RPC calls."""

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from threading import Thread
import unittest
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from .local_readonly_rpc_proxy import handler_for, read_upstream


class LocalRpcProxyTests(unittest.TestCase):
    def test_only_https_upstream_configuration(self):
        with TemporaryDirectory() as directory:
            path = Path(directory) / "rpc.env"
            path.write_text("RPC_URL=http://example.test/rpc\n")
            with self.assertRaises(ValueError):
                read_upstream(path)
            path.write_text("RPC_URL=https://example.test/rpc\n")
            self.assertEqual(read_upstream(path), "https://example.test/rpc")

    def test_local_read_passes_but_transaction_submission_is_refused(self):
        seen = []

        class Upstream(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                seen.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
                body = b'{"jsonrpc":"2.0","id":1,"result":"0x14a34"}'
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        upstream = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
        proxy = ThreadingHTTPServer(("127.0.0.1", 0), handler_for(f"http://127.0.0.1:{upstream.server_port}"))
        threads = [Thread(target=server.serve_forever, daemon=True) for server in (upstream, proxy)]
        for thread in threads:
            thread.start()
        try:
            def call(method):
                body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": []}).encode()
                return urlopen(Request(f"http://127.0.0.1:{proxy.server_port}", data=body), timeout=2)

            with call("eth_chainId") as response:
                self.assertEqual(json.load(response)["result"], "0x14a34")
            with self.assertRaises(HTTPError) as denied:
                call("eth_sendRawTransaction")
            self.assertEqual(denied.exception.code, 403)
            self.assertEqual([request["method"] for request in seen], ["eth_chainId"])
        finally:
            proxy.shutdown()
            upstream.shutdown()
            proxy.server_close()
            upstream.server_close()
            for thread in threads:
                thread.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
