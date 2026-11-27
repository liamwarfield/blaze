"""Extra functions for scripts/rrc-probe (RRC_EXTRA=scripts/rrc-probe/extra_round7.py).

Two hand-written cases from review 2026-11-19 round 7 (not in Bazel's test file), run the same
way on the oracle and on Blaze; they assert only what both sides must share, and log every
command, so the logs are compared by hand:

  testRound7LostBlobMidCommand (N1): `build //:x` after `clean --expunge`, where //:BUILD.bazel
    loads @my_repo//:defs.bzl (so @my_repo is served from the cache and injected first) and
    @slow//:defs.bzl, whose (uncached) repository rule waits 3 s and then deletes the blob of
    @my_repo's sub/BUILD from the worker's CAS; //:x then needs @my_repo//sub, whose BUILD file
    is no longer in the cache. The read fails with REMOTE_FILE_EVICTED, the build is retried
    ("Found transient remote cache error, retrying the build..."), @my_repo is fetched again (the
    cache lookup misses now: the worker checks the outputs of an action result) and the build
    succeeds. (Deleting the blob before the command is no case: the lookup misses at once.)
  testRound7LostBlobBeforeTargetPatterns (N1, the retry): `build --@my_repo//flags:f=1
    @my_repo//sub:sub` after `clean --expunge`. Parsing the Starlark flag loads @my_repo//flags
    (so @my_repo is served from the cache and injected) before the target patterns are read;
    flags/BUILD loads @slow//:defs.bzl, whose (uncached) repository rule deletes the blob of
    sub/BUILD from the worker's CAS. The pattern's package then fails with REMOTE_FILE_EVICTED
    (exit code 39), the build is retried and succeeds with @my_repo fetched again.
  testRound7QueryWithoutCacheAfterCachedBuild (N2): `build @my_repo//:root` served from the
    cache, then on the same server `query --remote_cache= @my_repo//sub:all`: the command has no
    cache, so the in-memory repository is evicted and fetched to disk.
"""

import hashlib
import os

REPO_BZL = [
    'def _repo_impl(rctx):',
    '  rctx.file("BUILD", "filegroup(name=\'root\', srcs=[\'root.txt\'])")',
    '  rctx.file("root.txt", "root")',
    '  rctx.file("sub/BUILD", "filegroup(name=\'sub\', srcs=[\'sub.txt\'])")',
    '  rctx.file("sub/sub.txt", "sub")',
    '  rctx.file("defs.bzl", "A = 1")',
    '  print("JUST FETCHED")',
    '  return rctx.repo_metadata(reproducible=True)',
    'repo = repository_rule(_repo_impl)',
]

RETRY = 'Found transient remote cache error, retrying the build...'


def _set_up(self):
    self.ScratchFile('MODULE.bazel', [
        'repo = use_repo_rule("//:repo.bzl", "repo")',
        'repo(name = "my_repo")',
    ])
    self.ScratchFile('BUILD.bazel')
    self.ScratchFile('repo.bzl', REPO_BZL)
    _, _, stderr = self.RunBazel(['build', '@my_repo//:root'])
    self.assertIn('JUST FETCHED', '\n'.join(stderr))
    self.RunBazel(['clean', '--expunge'])


def _lost_blob_mid_command(self, command):
    _set_up(self)
    content = b"filegroup(name='sub', srcs=['sub.txt'])"
    digest = hashlib.sha256(content).hexdigest()
    blob = os.path.join(self._cas_path, "cas", digest[:2], digest)
    self.assertTrue(os.path.exists(blob))
    self.ScratchFile('MODULE.bazel', [
        'repo = use_repo_rule("//:repo.bzl", "repo")',
        'repo(name = "my_repo")',
        'slow = use_repo_rule("//:slow.bzl", "slow")',
        'slow(name = "slow")',
    ])
    self.ScratchFile('slow.bzl', [
        'def _slow_impl(rctx):',
        '  rctx.execute(["sleep", "3"])',
        '  rctx.execute(["rm", "-f", "%s"])' % blob,
        '  rctx.file("BUILD", "")',
        '  rctx.file("defs.bzl", "B = 2")',
        '  print("SLOW FETCHED")',
        'slow = repository_rule(_slow_impl)',
    ])
    self.ScratchFile('BUILD.bazel', [
        'load("@my_repo//:defs.bzl", "A")',
        'load("@slow//:defs.bzl", "B")',
        'filegroup(name = "x", srcs = ["@my_repo//sub:sub"])',
    ])
    exit_code, stdout, stderr = self.RunBazel(command, allow_failure=True)
    stderr = '\n'.join(stderr)
    print('exit %d, retries %d, lost-file messages %d, my_repo fetches %d, slow fetches %d, '
          'stdout %r' % (
              exit_code, stderr.count(RETRY),
              stderr.count('no longer available in the remote cache'),
              stderr.count('JUST FETCHED'), stderr.count('SLOW FETCHED'), stdout))
    self.assertFalse(os.path.exists(blob))
    self.assertRegex(stderr, 'external/[^/]*my_repo/sub/BUILD with digest .*/.* no longer'
                     ' available in the remote cache')


class Extra:

    def testRound7LostBlobBeforeTargetPatterns(self):
        content = b"filegroup(name='sub', srcs=['sub.txt'])"
        digest = hashlib.sha256(content).hexdigest()
        blob = os.path.join(self._cas_path, "cas", digest[:2], digest)
        self.ScratchFile('MODULE.bazel', [
            'repo = use_repo_rule("//:repo.bzl", "repo")',
            'repo(name = "my_repo")',
            'slow = use_repo_rule("//:slow.bzl", "slow")',
            'slow(name = "slow")',
        ])
        self.ScratchFile('BUILD.bazel')
        self.ScratchFile('slow.bzl', [
            'def _slow_impl(rctx):',
            '  rctx.execute(["rm", "-f", "%s"])' % blob,
            '  rctx.file("BUILD", "")',
            '  rctx.file("defs.bzl", "B = 2")',
            '  print("SLOW FETCHED")',
            'slow = repository_rule(_slow_impl)',
        ])
        self.ScratchFile('repo.bzl', REPO_BZL[:-2] + [
            '  rctx.file("flag.bzl", "def _impl(ctx):\\n  return []\\n'
            'flag = rule(implementation = _impl, build_setting = config.int(flag = True))\\n")',
            '  rctx.file("flags/BUILD", "load(\'@slow//:defs.bzl\', \'B\')\\n'
            'load(\'//:flag.bzl\', \'flag\')\\nflag(name = \'f\', build_setting_default = 0)\\n")',
        ] + REPO_BZL[-2:])
        _, _, stderr = self.RunBazel(['build', '@my_repo//:root'])
        self.assertIn('JUST FETCHED', '\n'.join(stderr))
        self.RunBazel(['clean', '--expunge'])
        self.assertTrue(os.path.exists(blob))
        exit_code, _, stderr = self.RunBazel(
            ['build', '--@my_repo//flags:f=1', '@my_repo//sub:sub'], allow_failure=True)
        stderr = '\n'.join(stderr)
        print('exit %d, retries %d, lost-file messages %d, my_repo fetches %d, slow fetches %d' % (
            exit_code, stderr.count(RETRY),
            stderr.count('no longer available in the remote cache'),
            stderr.count('JUST FETCHED'), stderr.count('SLOW FETCHED')))
        self.assertRegex(stderr, 'external/[^/]*my_repo/sub/BUILD with digest .*/.* no longer'
                         ' available in the remote cache')
        self.assertEqual(1, stderr.count(RETRY))
        self.assertEqual(0, exit_code)

    def testRound7LostBlobMidBuild(self):
        # The oracle: an analysis failure (exit 1), no retry.
        _lost_blob_mid_command(self, ['build', '//:x'])

    def testRound7LostBlobMidQuery(self):
        _lost_blob_mid_command(self, ['query', 'deps(//:x)'])

    def testRound7QueryWithoutCacheAfterCachedBuild(self):
        _set_up(self)
        _, _, stderr = self.RunBazel(['build', '@my_repo//:root'])
        self.assertNotIn('JUST FETCHED', '\n'.join(stderr))
        exit_code, stdout, stderr = self.RunBazel(
            ['query', '--remote_cache=', '@my_repo//sub:all'], allow_failure=True)
        print('query exit %d, stdout %r, fetched %s' % (
            exit_code, stdout, 'JUST FETCHED' in '\n'.join(stderr)))
        self.assertEqual(0, exit_code)
        self.assertIn('@my_repo//sub:sub', stdout)
