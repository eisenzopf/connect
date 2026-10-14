"""Expiry probe must respect admission-versus-current-execution authority."""
import json
import pathlib
import sys
import unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from lib.grant_boundary import status_at_boundary


def response(error=False, code='forbidden', task='task'):
    return 200,json.dumps({'jsonrpc':'2.0','id':1,'result':{'isError':error,
        'structuredContent':{'code':code} if error else {'data':{'id':task}}}}).encode()


class GrantBoundary(unittest.TestCase):
    def classify(self,value,start=9,finish=11):
        return status_at_boundary(value,start,finish,10,'task')

    def test_completed_before_expiry_or_response_crossing_it(self):
        self.assertEqual(self.classify(response(),8,9),'status-completed')
        self.assertEqual(self.classify(response()),'status-completed')

    def test_http_denial_after_expiry(self):
        self.assertEqual(self.classify((401,b''),10,11),'http-unauthorized')
        self.assertEqual(self.classify((401,b'')),'http-unauthorized')

    def test_only_measured_crossing_can_explain_current_execution_denial(self):
        self.assertEqual(self.classify(response(True)),'execution-expired')
        self.assertEqual(self.classify(response(True),9,10),'execution-expired')

    def test_early_denial_and_unrelated_tool_errors_remain_failed(self):
        for value in (response(True),(401,b'')):
            with self.subTest(value=value),self.assertRaises(AssertionError):self.classify(value,8,9)
        for code in ('unauthorized','storage','unsupported-operation'):
            with self.subTest(code=code),self.assertRaises(AssertionError):self.classify(response(True,code))

    def test_fresh_post_expiry_success_or_forbidden_never_counts_as_race(self):
        for value in (response(),response(True)):
            with self.subTest(value=value),self.assertRaises(AssertionError):self.classify(value,10,11)

    def test_task_correlation_and_clock_shape_remain_required(self):
        with self.assertRaises(AssertionError):self.classify(response(task='different'))
        with self.assertRaises(AssertionError):self.classify(response(),11,9)
        with self.assertRaises(AssertionError):self.classify((500,b''))
        with self.assertRaises(AssertionError):self.classify((200,b'{"id":2}'))

if __name__=='__main__':unittest.main()
