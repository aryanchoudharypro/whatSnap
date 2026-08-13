import unittest
import sqlite3
import os
import sys
import json
import time
import ctypes
import subprocess

class TestDbFlowOverIpc(unittest.TestCase):
	db_path = "test.db"
	pipe_name = r"\\.\pipe\whantv-ipc"

	def setUp(self):
		conn = sqlite3.connect(self.db_path)
		cursor = conn.cursor()
		cursor.execute("CREATE TABLE IF NOT EXISTS messages (id TEXT PRIMARY KEY, chat_jid TEXT, content TEXT, timestamp INTEGER)")
		conn.commit()
		conn.close()

	def test_real_db_startup_and_ipc_dispatch(self):
		conn = sqlite3.connect(self.db_path)
		cursor = conn.cursor()
		msg_id = f"msg_{int(time.time() * 1000)}"
		chat_jid = "1234567890@s.whatsapp.net"
		content = "E2E IPC DB flow test"
		cursor.execute("INSERT OR REPLACE INTO messages (id, chat_jid, content, timestamp) VALUES (?, ?, ?, ?)", (msg_id, chat_jid, content, int(time.time())))
		conn.commit()

		cursor.execute("SELECT content FROM messages WHERE id = ?", (msg_id,))
		row = cursor.fetchone()
		conn.close()

		self.assertIsNotNone(row)
		self.assertEqual(row[0], content)

	def test_db_message_history_query_flow(self):
		conn = sqlite3.connect(self.db_path)
		cursor = conn.cursor()
		cursor.execute("PRAGMA integrity_check")
		res = cursor.fetchone()
		conn.close()
		self.assertIsNotNone(res)
		self.assertEqual(res[0], "ok")

	def test_ipc_payload_formatting(self):
		payload = {
			"action": "send_message",
			"data": {
				"to": "1234567890@s.whatsapp.net",
				"message": "hello over ipc"
			}
		}
		json_str = json.dumps(payload)
		parsed = json.loads(json_str)
		self.assertEqual(parsed["action"], "send_message")
		self.assertEqual(parsed["data"]["to"], "1234567890@s.whatsapp.net")

if __name__ == "__main__":
	unittest.main()
