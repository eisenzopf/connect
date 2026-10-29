import importlib.util
from pathlib import Path
import struct
import unittest
from connect_sdk import RtpPacket

spec=importlib.util.spec_from_file_location('packet_codec',Path(__file__).resolve().parents[2]/'examples/sdk-media/packet_codec.py')
codec=importlib.util.module_from_spec(spec);spec.loader.exec_module(codec)


def key():return b'\x00\x00\x00\x9d\x01\x2a\x80\x02\x68\x01'+b'*'*4000

def packets(data=None,timestamp=9000,sequence=65534,source=77):
    return [RtpPacket(i,p) for i,p in enumerate(codec.packetize_vp8(key() if data is None else data,timestamp,sequence,source))]


class PacketCodec(unittest.TestCase):
    def test_fragment_wrap_original_bytes_and_timestamp(self):
        a=codec.Vp8Assembler();p=packets();self.assertEqual([v.header.sequence_number for v in p],[65534,65535,0,1]);self.assertTrue(all(len(v.data)<=1180 for v in p))
        for v in p[:-1]:self.assertIsNone(a.push(v,0))
        self.assertEqual(a.push(p[-1],0.01),(9000,key()));self.assertIsNone(a.push(p[-1],0.011))

    def test_gap_timeout_suppress_dependent_frame_until_fresh_key(self):
        a=codec.Vp8Assembler();p=packets();a.push(p[0],0);self.assertIsNone(a.push(p[2],0.001));self.assertIsNone(a.push(p[3],0.002));self.assertIsNone(a.push(packets(b'\x01\x03\x04',18000,20)[0],0.003))
        p=packets(timestamp=27000,sequence=30);a.push(p[0],0.005)
        for v in p[1:]:self.assertIsNone(a.push(v,0.206))
        result=None
        for v in packets(timestamp=36000,sequence=40):result=a.push(v,0.21)
        self.assertEqual(result,(36000,key()))

    def test_bad_formats_source_and_frame_limits(self):
        for data in (b'',b' '* (codec.MAX_FRAME+1)):
            with self.assertRaises(ValueError):codec.packetize_vp8(data,0,0,1)
        a=codec.Vp8Assembler();a.push(packets()[0],0)
        with self.assertRaises(ValueError):a.push(packets(sequence=65535,source=88)[1],0.001)
        bad=bytearray(packets()[0].data);bad[12]=0x80
        with self.assertRaises(ValueError):codec.Vp8Assembler().push(RtpPacket(1,bytes(bad)))
        bounded=codec.Vp8Assembler()
        with self.assertRaises(ValueError):
            for i in range(226):bounded.push(RtpPacket(i,codec.rtp(96,i,0,99,bytes([0x10 if i==0 else 0])+b'\x00'*1167)),0)
        for p in packets(timestamp=9000,sequence=300,source=99):recovered=bounded.push(p,0.001)
        self.assertEqual(recovered,(9000,key()))
        wrong=bytearray(key());wrong[6]=0;a=codec.Vp8Assembler()
        with self.assertRaises(ValueError):
            for p in packets(bytes(wrong)):a.push(p,0)

    def test_ivf_original_pts_and_closed_framing(self):
        frame=(9000,key());data=codec.ivf_header()+codec.frame_record(frame,0);self.assertEqual(codec.parse_ivf(data),[frame])
        for bad in (data[:-1],data+b'\x00',b'\x00'*32):
            with self.assertRaises(ValueError):codec.parse_ivf(bad)
        bad=bytearray(data);struct.pack_into('<I',bad,24,2)
        with self.assertRaises(ValueError):codec.parse_ivf(bad)
        struct.pack_into('<I',bad,24,1);bad[8]=0xd6
        with self.assertRaises(ValueError):codec.parse_ivf(bad)


if __name__=='__main__':unittest.main()
