// Writes the expected outputs of tests/synthetic.rs using the protobuf runtime and the generated
// message classes inside the oracle's server jar (Bazel 9.3.2). Run by scripts/record-synthetic.
import com.google.devtools.build.lib.actions.extra.CppCompileInfo;
import com.google.devtools.build.lib.actions.extra.DetailedExtraActionInfo;
import com.google.devtools.build.lib.actions.extra.EnvironmentVariable;
import com.google.devtools.build.lib.actions.extra.ExtraActionInfo;
import com.google.devtools.build.lib.actions.extra.ExtraActionSummary;
import com.google.devtools.build.lib.actions.extra.PythonInfo;
import com.google.devtools.build.lib.actions.extra.SpawnInfo;
import com.google.devtools.build.lib.buildeventstream.BuildEventStreamProtos.ActionExecuted;
import com.google.devtools.build.lib.buildeventstream.BuildEventStreamProtos.BuildEvent;
import com.google.devtools.build.lib.buildeventstream.BuildEventStreamProtos.BuildMetadata;
import com.google.devtools.build.lib.buildeventstream.BuildEventStreamProtos.BuildMetrics;
import com.google.devtools.build.lib.exec.Protos.Digest;
import com.google.devtools.build.lib.exec.Protos.File;
import com.google.devtools.build.lib.exec.Protos.SpawnExec;
import com.google.protobuf.Any;
import com.google.protobuf.Duration;
import com.google.protobuf.Message;
import com.google.protobuf.TextFormat;
import com.google.protobuf.Timestamp;
import com.google.protobuf.util.JsonFormat;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

public class JavaSynthetic {
  static Path out;

  static void bin(String name, Message m) throws Exception {
    Files.write(out.resolve(name + ".bin"), m.toByteArray());
  }

  static void text(String name, String s) throws Exception {
    Files.write(out.resolve(name), s.getBytes(StandardCharsets.UTF_8));
  }

  public static void main(String[] args) throws Exception {
    out = Path.of(args[0]);
    JsonFormat.Printer pretty = JsonFormat.printer();
    JsonFormat.Printer compact = JsonFormat.printer().omittingInsignificantWhitespace();
    JsonFormat.Printer always = JsonFormat.printer().alwaysPrintFieldsWithNoPresence();
    JsonFormat.TypeRegistry registry =
        JsonFormat.TypeRegistry.newBuilder().add(SpawnExec.getDescriptor()).build();

    // A oneof member (24) after a plain field (20).
    BuildEvent oneof =
        BuildEvent.newBuilder()
            .setLastMessage(true)
            .setBuildMetrics(BuildMetrics.getDefaultInstance())
            .build();
    bin("oneof_order", oneof);
    text("oneof_order.json", pretty.print(oneof));

    // Map entries: default key and value are still written; insertion order is kept.
    BuildMetadata map =
        BuildMetadata.newBuilder()
            .putMetadata("z", "1")
            .putMetadata("", "")
            .putMetadata("a=b", "<&>'é \n")
            .build();
    bin("map_entries", map);
    text("map_entries.json", pretty.print(map));
    text("map_entries.compact.json", compact.print(map));
    text("map_empty.always.json", always.print(BuildMetadata.getDefaultInstance()));
    text("map_empty.json", pretty.print(BuildMetadata.getDefaultInstance()));

    // Every no-presence field of SpawnExec printed at its default, plus nested messages.
    SpawnExec exec =
        SpawnExec.newBuilder()
            .addInputs(File.newBuilder().setPath("p").setDigest(Digest.getDefaultInstance()))
            .addInputs(File.getDefaultInstance())
            .build();
    text("spawn_exec.always.json", always.print(exec));
    text("spawn_exec_default.always.json", always.print(SpawnExec.getDefaultInstance()));

    // Any: a registered message, well-known types, and the empty Any.
    SpawnExec packed =
        SpawnExec.newBuilder()
            .addCommandArgs("echo")
            .setTimeoutMillis(-5)
            .setExitCode(-1)
            .setMnemonic("M")
            .build();
    ActionExecuted any =
        ActionExecuted.newBuilder()
            .addStrategyDetails(Any.pack(packed))
            .addStrategyDetails(
                Any.pack(Duration.newBuilder().setSeconds(-1).setNanos(-500000000).build()))
            .addStrategyDetails(
                Any.pack(Timestamp.newBuilder().setSeconds(1).setNanos(1000).build()))
            .addStrategyDetails(Any.getDefaultInstance())
            .setStartTime(Timestamp.newBuilder().setSeconds(1700000000).setNanos(120000000))
            .build();
    bin("any", any);
    text("any.json", pretty.usingTypeRegistry(registry).print(any));
    text("any.compact.json", compact.usingTypeRegistry(registry).print(any));
    String error;
    try {
      pretty.print(any);
      error = "no error";
    } catch (Exception e) {
      error = e.getClass().getName() + ": " + e.getMessage();
    }
    text("any.unregistered.error", error);

    // Extra actions: ExtraActionInfo with extensions (written after the fields, in number
    // order) as print_action prints them (TextFormat.printer().printToString(summary)).
    ExtraActionInfo spawn =
        ExtraActionInfo.newBuilder()
            .setOwner("//pkg:gen")
            .setId("0123abcd")
            .setMnemonic("Genrule")
            .setAspectName("MyAspect")
            .putAspectParameters(
                "p", ExtraActionInfo.StringList.newBuilder().addValue("v1").addValue("v2").build())
            .setExtension(
                SpawnInfo.spawnInfo,
                SpawnInfo.newBuilder()
                    .addArgument("/bin/bash")
                    .addArgument("-c")
                    .addArgument("echo \"é\" > $@")
                    .addVariable(EnvironmentVariable.newBuilder().setName("PATH").setValue("/bin"))
                    .addInputFile("pkg/in.txt")
                    .addOutputFile("bazel-out/k8-fastbuild/bin/pkg/out.txt")
                    .build())
            .build();
    ExtraActionInfo cpp =
        ExtraActionInfo.newBuilder()
            .setOwner("//pkg:lib")
            .setId("ffff")
            .setMnemonic("CppCompile")
            .setExtension(
                PythonInfo.pythonInfo, PythonInfo.newBuilder().addSourceFile("a.py").build())
            .setExtension(
                CppCompileInfo.cppCompileInfo,
                CppCompileInfo.newBuilder()
                    .setTool("/usr/bin/gcc")
                    .addCompilerOption("-c")
                    .setSourceFile("pkg/a.cc")
                    .setOutputFile("bazel-out/k8-fastbuild/bin/pkg/_objs/lib/a.o")
                    .addSourcesAndHeaders("pkg/a.cc")
                    .build())
            .build();
    ExtraActionSummary summary =
        ExtraActionSummary.newBuilder()
            .addAction(DetailedExtraActionInfo.newBuilder().setAction(spawn))
            .addAction(
                DetailedExtraActionInfo.newBuilder().setTriggeringFile("pkg/a.cc").setAction(cpp))
            .build();
    bin("extra_action_info", spawn);
    bin("extra_action_summary", summary);
    text("extra_action_summary.txt", TextFormat.printer().printToString(summary));
  }
}
