from pathlib import Path
import importlib.util,json,os,shutil,subprocess,tempfile,unittest
spec=importlib.util.spec_from_file_location('ci_cache',Path(__file__).resolve().parents[1] / 'lib/ci_dependency_cache.py');CACHE=importlib.util.module_from_spec(spec);spec.loader.exec_module(CACHE)
LOCK={'sources':{'rvoip':{'revision':'fixed'}},'patches':[{'path':'recorded.patch'}]}
class Cache(unittest.TestCase):
 def root(self,d):
  r=Path(d);(r/'dependencies').mkdir();(r/'dependencies/source-lock.json').write_text(json.dumps(LOCK));return r
 def marker(self,r,lock):
  p=r/'.deps/rvoip/.git';p.mkdir(parents=True);(p/'connect-prepared.json').write_text(json.dumps(lock))
 def test_compatible_cache_still_runs_the_strict_preparer(self):
  with tempfile.TemporaryDirectory() as d:
   r=self.root(d);self.marker(r,LOCK);commands=[];v=CACHE.prepare(r,commands.append)
   self.assertEqual(commands,[['bash','scripts/prepare-dependencies.sh']]);self.assertFalse(v['retained_incompatible_cache'])
 def test_changed_identity_is_retained_and_only_identical_current_source_times_return(self):
  with tempfile.TemporaryDirectory() as d:
   r=self.root(d);self.marker(r,{'older':True});old=r/'.deps/same.rs';old.write_text('same');os.utime(old,ns=(2_000_000_000,2_000_000_000));(r/'.deps/changed.rs').write_text('old')
   def run(command):
    self.assertEqual(command[:3],['bash','scripts/prepare-dependencies.sh','--cache']);p=r/'.deps';p.mkdir();(p/'same.rs').write_text('same');os.utime(p/'same.rs',ns=(4_000_000_000,4_000_000_000));(p/'changed.rs').write_text('current')
   v=CACHE.prepare(r,run);self.assertTrue(v['retained_incompatible_cache']);self.assertEqual(v['restored_identical_files'],1);self.assertEqual((r/'.deps/same.rs').stat().st_mtime_ns,2_000_000_000);self.assertEqual((r/'.deps/changed.rs').read_text(),'current');self.assertTrue(list((r/'.local/gate6-linux-ci/retained-deps').glob('*/changed.rs')))
 def test_selected_dependency_symlink_is_never_replaced(self):
  with tempfile.TemporaryDirectory() as d:
   r=self.root(d);outside=r/'selected';outside.mkdir();(r/'.deps').symlink_to(outside,target_is_directory=True)
   with self.assertRaises(ValueError):CACHE.prepare(r,lambda _:self.fail('preparer must not run'))
   self.assertTrue((r/'.deps').is_symlink())
 def test_failed_preparation_preserves_old_cache_and_cannot_restore_times(self):
  with tempfile.TemporaryDirectory() as d:
   r=self.root(d);self.marker(r,{'older':True});(r/'.deps/old.rs').write_text('old')
   def run(_):raise RuntimeError('expected')
   with self.assertRaises(RuntimeError):CACHE.prepare(r,run)
   self.assertFalse((r/'.deps').exists());self.assertEqual(len(list((r/'.local/gate6-linux-ci/retained-deps').glob('*/old.rs'))),1)
 def test_timestamps_never_follow_symlinks_or_change_different_modes_or_contents(self):
  with tempfile.TemporaryDirectory() as d:
   r=Path(d);old=r/'old';new=r/'new';old.mkdir();new.mkdir();outside=r/'outside';outside.mkdir();(outside/'secret.rs').write_text('same');(old/'link').symlink_to(outside,target_is_directory=True);(new/'link').mkdir();(new/'link/secret.rs').write_text('same')
   (old/'mode.rs').write_text('same');(new/'mode.rs').write_text('same');(old/'mode.rs').chmod(0o600);(new/'mode.rs').chmod(0o644);(old/'different.rs').write_text('old');(new/'different.rs').write_text('new');v=CACHE.restore_identical_times(new,old)
   self.assertEqual(v['restored_identical_files'],0);self.assertEqual((new/'different.rs').read_text(),'new');self.assertEqual((new/'mode.rs').stat().st_mode&0o777,0o644)
class PinnedPreparation(unittest.TestCase):
 def test_changed_patch_is_freshly_applied_by_the_real_preparer(self):
  with tempfile.TemporaryDirectory() as directory:
   root=Path(directory);upstream=root/'upstream';upstream.mkdir()
   def git(*args):
    return subprocess.check_output(['git','-C',str(upstream),*args],stderr=subprocess.DEVNULL,text=True).strip()
   git('init','--quiet');git('config','user.name','Synthetic fixture');git('config','user.email','fixture@example.invalid')
   (upstream/'changed.rs').write_text('base\n');(upstream/'same.rs').write_text('unchanged\n')
   git('add','.');git('commit','--quiet','-m','Synthetic pinned input');revision=git('rev-parse','HEAD')
   (root/'scripts/lib').mkdir(parents=True);(root/'dependencies').mkdir();(root/'patches').mkdir()
   source=Path(__file__).resolve().parents[1]
   shutil.copyfile(source/'prepare-dependencies.py',root/'scripts/prepare-dependencies.py')
   shutil.copyfile(source/'lib/prepared_archive.py',root/'scripts/lib/prepared_archive.py')
   (root/'scripts/prepare-dependencies.sh').write_text('#!/bin/sh\nexec python3 scripts/prepare-dependencies.py "$@"\n')
   def lock(value):
    patch=root/'patches/current.patch';patch.write_text('--- a/changed.rs\n+++ b/changed.rs\n@@ -1 +1 @@\n-base\n+'+value+'\n')
    data={'sources':{'rvoip':{'revision':revision,'url':str(upstream)}},'patches':[{'path':'patches/current.patch','sha256':CACHE.hashlib.sha256(patch.read_bytes()).hexdigest()}]}
    (root/'dependencies/source-lock.json').write_text(json.dumps(data))
   def run(command):
    subprocess.run(command,cwd=root,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
   lock('older');CACHE.prepare(root,run)
   os.utime(root/'.deps/rvoip/same.rs',ns=(2_000_000_000,2_000_000_000))
   lock('current');report=CACHE.prepare(root,run)
   self.assertTrue(report['retained_incompatible_cache'])
   self.assertGreaterEqual(report['restored_identical_files'],1)
   self.assertEqual((root/'.deps/rvoip/changed.rs').read_text(),'current\n')
   self.assertEqual((root/'.deps/rvoip/same.rs').stat().st_mtime_ns,2_000_000_000)
   self.assertEqual(list((root/'.local/gate6-linux-ci/retained-deps').glob('*/rvoip/changed.rs'))[0].read_text(),'older\n')
   # Fresh pinned preparation remains reproducible and passes its own full
   # patch/content/status verification after timestamp reuse.
   self.assertFalse(CACHE.prepare(root,run)['retained_incompatible_cache'])
if __name__=='__main__':unittest.main()
